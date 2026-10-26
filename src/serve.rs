use crate::config::Config;
use crate::content::Drafts;
use crate::site::{self, BuildOptions};
use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use tiny_http::{Header, Request, Response, Server};

const LIVE_SCRIPT: &str = r"<script>(()=>{let v=null;setInterval(async()=>{try{const t=await (await fetch('/__agno/live',{cache:'no-store'})).text();if(v===null)v=t;else if(v!==t)location.reload()}catch(e){}},400)})();</script>";

/// Build the site, serve it on `port`, and rebuild on every change under
/// `root`.
///
/// # Errors
///
/// Fails when the first build fails, the port cannot be bound, or the
/// filesystem watcher cannot be created.
pub fn dev(root: &Path, port: u16, drafts: Drafts) -> Result<()> {
    // Watch paths come back relative to the directory handed to `watch`, so a
    // canonical root keeps them comparable with the output directory.
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;
    let opts = BuildOptions {
        drafts: drafts.includes(),
        clean: true,
        minify: false,
    };
    let stats = site::build(&root, &opts)?;
    println!(
        "agno dev → http://localhost:{port}  ({} pages, {} ms)",
        stats.pages,
        stats.total.as_millis()
    );

    let cfg = Config::load(&root)?;
    let dist = cfg.output();
    let server = Arc::new(
        Server::http(("127.0.0.1", port))
            .map_err(|err| anyhow::anyhow!("starting server: {err}"))?,
    );
    let version = Arc::new(AtomicU64::new(0));

    for _ in 0..4 {
        let server = server.clone();
        let dist = dist.clone();
        let version = version.clone();
        std::thread::spawn(move || serve_loop(server, dist, version));
    }

    let (tx, rx) = mpsc::channel();
    // Watch the whole site rather than the three input directories that happen
    // to exist at startup: a `templates/` or `static/` directory created later
    // used to be invisible until the server restarted. Build output and the
    // fingerprint cache are filtered out, otherwise every write would schedule
    // another build.
    let ignored = [dist.clone(), root.join(".agno-cache")];
    let watched_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = &event {
            let interesting =
                event.kind.is_create() || event.kind.is_modify() || event.kind.is_remove();
            if !interesting
                || event
                    .paths
                    .iter()
                    .all(|path| is_ignored(path, &watched_root, &ignored))
            {
                return;
            }
        }
        let _ = tx.send(event);
    })
    .context("creating file watcher")?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("watching {}", root.display()))?;

    loop {
        match rx.recv() {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                eprintln!("watch error: {err}");
                continue;
            }
            Err(_) => break,
        }
        while rx.recv_timeout(Duration::from_millis(120)).is_ok() {}

        let started = std::time::Instant::now();
        match site::build(
            &root,
            &BuildOptions {
                drafts: drafts.includes(),
                clean: false,
                minify: false,
            },
        ) {
            Ok(_) => {
                version.fetch_add(1, Ordering::SeqCst);
                println!("rebuilt in {} ms", started.elapsed().as_millis());
            }
            Err(err) => eprintln!("build failed: {err:#}"),
        }
    }
    Ok(())
}

/// Build output, the fingerprint cache and dot-directories (`.git`, editor swap
/// files) never trigger a rebuild.
fn is_ignored(path: &Path, root: &Path, ignored: &[PathBuf]) -> bool {
    if ignored.iter().any(|dir| path.starts_with(dir)) {
        return true;
    }
    path.strip_prefix(root).is_ok_and(|rel| {
        rel.components()
            .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
    })
}

// The three values are moved into the thread that runs this loop, so taking
// them by reference would need a lifetime the spawn cannot provide.
#[allow(clippy::needless_pass_by_value)]
fn serve_loop(server: Arc<Server>, dist: PathBuf, version: Arc<AtomicU64>) {
    while let Ok(request) = server.recv() {
        let url = request.url().to_string();
        let path = url.split(['?', '#']).next().unwrap_or("/");

        if path == "/__agno/live" {
            respond(
                request,
                200,
                "text/plain; charset=utf-8",
                version.load(Ordering::SeqCst).to_string(),
            );
            continue;
        }

        match resolve(&dist, path) {
            Some(Target::Redirect(location)) => {
                // Preserve the query string: `/blog?x=1` → `/blog/?x=1`.
                let location = match url.split_once('?') {
                    Some((_, query)) => format!("{location}?{query}"),
                    None => location,
                };
                let response = Response::empty(301)
                    .with_header(header("Location", &location))
                    .with_header(header("Cache-Control", "no-store"));
                let _ = request.respond(response);
            }
            Some(Target::File(file)) if file.extension().is_some_and(|e| e == "html") => {
                match std::fs::read_to_string(&file) {
                    Ok(html) => respond(request, 200, "text/html; charset=utf-8", inject(&html)),
                    Err(_) => respond(
                        request,
                        500,
                        "text/plain; charset=utf-8",
                        "read error".into(),
                    ),
                }
            }
            Some(Target::File(file)) => match std::fs::read(&file) {
                Ok(bytes) => {
                    let ctype = content_type(&file);
                    let response = Response::from_data(bytes)
                        .with_status_code(200)
                        .with_header(header("Content-Type", ctype))
                        .with_header(header("Cache-Control", "no-store"));
                    let _ = request.respond(response);
                }
                Err(_) => respond(
                    request,
                    500,
                    "text/plain; charset=utf-8",
                    "read error".into(),
                ),
            },
            None => {
                let not_found = dist.join("404.html");
                let body = std::fs::read_to_string(&not_found)
                    .map_or_else(|_| "<h1>404</h1>".to_string(), |html| inject(&html));
                respond(request, 404, "text/html; charset=utf-8", body);
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Target {
    File(PathBuf),
    Redirect(String),
}

fn resolve(dist: &Path, url_path: &str) -> Option<Target> {
    let url_path = url_path.split(['?', '#']).next().unwrap_or("");
    let decoded = site::percent_decode(url_path)?;
    if decoded.contains(['\\', '\0']) {
        return None;
    }
    // Reject `..` as a path segment, not as a substring: a file called
    // `a..b.txt` is a perfectly ordinary name.
    if decoded.split('/').any(|segment| segment == "..") {
        return None;
    }
    let rel = decoded.trim_start_matches('/');
    let base = dist.join(rel);
    if base.is_dir() {
        // Browsers resolve relative links against the directory URL, so a
        // request for `/blog` must move to `/blog/` before the page is served.
        if !decoded.ends_with('/') {
            return Some(Target::Redirect(format!("{decoded}/")));
        }
        let index = base.join("index.html");
        return index.is_file().then_some(Target::File(index));
    }
    if base.is_file() {
        return Some(Target::File(base));
    }
    None
}

fn inject(html: &str) -> String {
    match html.rfind("</body>") {
        Some(index) => format!("{}{}{}", &html[..index], LIVE_SCRIPT, &html[index..]),
        None => format!("{html}{LIVE_SCRIPT}"),
    }
}

fn respond(request: Request, status: u16, content_type: &str, body: String) {
    let response = Response::from_string(body)
        .with_status_code(status)
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"));
    let _ = request.respond(response);
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header values are valid")
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn serves_webassembly_and_source_maps() {
        assert_eq!(content_type(Path::new("app.wasm")), "application/wasm");
        assert_eq!(
            content_type(Path::new("app.js.map")),
            "application/json; charset=utf-8"
        );
    }

    #[test]
    fn percent_decode_handles_spaces_and_rejects_traversal() {
        assert_eq!(
            site::percent_decode("/hello%20world/").as_deref(),
            Some("/hello world/")
        );
        assert_eq!(
            site::percent_decode("/%2e%2e/secret").as_deref(),
            Some("/../secret")
        );
        assert_eq!(site::percent_decode("/bad%zz"), None);
    }

    #[test]
    fn resolve_decodes_paths_and_stays_inside_dist() {
        let dist = std::env::temp_dir().join(format!("agno-serve-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dist);
        fs::create_dir_all(dist.join("hello world")).unwrap();
        fs::write(dist.join("hello world/index.html"), "hi").unwrap();
        fs::write(dist.join("secret.txt"), "secret").unwrap();
        fs::write(dist.join("a..b.txt"), "dots").unwrap();

        assert_eq!(
            resolve(&dist, "/hello%20world/"),
            Some(Target::File(dist.join("hello world/index.html")))
        );
        assert_eq!(
            resolve(&dist, "/hello%20world"),
            Some(Target::Redirect("/hello world/".into())),
            "a directory without a trailing slash redirects"
        );
        assert_eq!(resolve(&dist, "/../secret.txt"), None);
        assert_eq!(resolve(&dist, "/%2e%2e/secret.txt"), None);
        assert_eq!(
            resolve(&dist, "/a..b.txt"),
            Some(Target::File(dist.join("a..b.txt"))),
            "dots inside a file name are not traversal"
        );

        let _ = fs::remove_dir_all(&dist);
    }

    #[test]
    fn ignores_output_cache_and_dot_directories() {
        let root = Path::new("/site");
        let ignored = [root.join("dist"), root.join(".agno-cache")];

        assert!(is_ignored(&root.join("dist/index.html"), root, &ignored));
        assert!(is_ignored(
            &root.join(".agno-cache/fingerprints.bin"),
            root,
            &ignored
        ));
        assert!(is_ignored(&root.join(".git/HEAD"), root, &ignored));
        assert!(is_ignored(
            &root.join("content/.hello.md.swp"),
            root,
            &ignored
        ));
        assert!(!is_ignored(
            &root.join("content/blog/hello.md"),
            root,
            &ignored
        ));
        assert!(!is_ignored(root, root, &ignored));
    }
}
