//! Conservative HTML minification.
//!
//! Whitespace and comments are the only things removed. A run of whitespace
//! collapses to a single space, and that space is dropped entirely when it sits
//! next to a block-level tag, where it can never affect layout. Inline spacing
//! such as `<span>a</span> <span>b</span>` is preserved.

/// Elements whose content must be copied byte for byte.
const RAW_ELEMENTS: [&str; 4] = ["pre", "textarea", "script", "style"];

/// Elements that start or end a block box; whitespace next to them is inert.
const BLOCK_ELEMENTS: [&str; 47] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "dd",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "li",
    "main",
    "meta",
    "link",
    "nav",
    "ol",
    "p",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "ul",
    "video",
    "audio",
];

struct Tag<'a> {
    name: &'a str,
}

pub fn minify_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut pending_space = false;
    let mut after_block = false;

    while index < bytes.len() {
        if bytes[index] == b'<' {
            if let Some(skip) = comment_end(input, index) {
                index = skip;
                continue;
            }
            if let Some(end) = raw_element_end(input, index) {
                flush_space(&mut out, &mut pending_space, after_block);
                out.push_str(&input[index..end]);
                index = end;
                after_block = false;
                continue;
            }
            if let Some((tag, end)) = parse_tag(input, index) {
                let block = is_block(tag.name);
                flush_space(&mut out, &mut pending_space, after_block || block);
                out.push_str(&input[index..end]);
                index = end;
                after_block = block;
                continue;
            }
        }

        if bytes[index].is_ascii_whitespace() {
            pending_space = true;
            index += 1;
            continue;
        }

        flush_space(&mut out, &mut pending_space, after_block);
        let ch = input[index..].chars().next().expect("valid utf-8 boundary");
        out.push(ch);
        index += ch.len_utf8();
        after_block = false;
    }

    out
}

fn flush_space(out: &mut String, pending: &mut bool, drop: bool) {
    if *pending && !drop && !out.is_empty() && !out.ends_with(' ') {
        out.push(' ');
    }
    *pending = false;
}

fn is_block(name: &str) -> bool {
    BLOCK_ELEMENTS.contains(&name)
}

fn parse_tag(input: &str, index: usize) -> Option<(Tag<'_>, usize)> {
    let rest = &input[index + 1..];
    let first = rest.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '/' || first == '!') {
        return None;
    }
    let end = tag_end(input, index)?;
    let mut body = &input[index + 1..end - 1];
    body = body.trim_start_matches('/');
    let name_end = body
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .unwrap_or(body.len());
    Some((
        Tag {
            name: &body[..name_end],
        },
        end,
    ))
}

/// Offset just past the `>` that closes the tag starting at `index`, ignoring
/// `>` characters inside quoted attribute values.
///
/// `title="a > b"` is a valid attribute, and ending the tag at the first `>`
/// would hand the rest of the value to the whitespace collapser, which then ate
/// the space after it.
fn tag_end(input: &str, index: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut cursor = index + 1;
    let mut quote: Option<u8> = None;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        match quote {
            Some(open) => {
                if byte == open {
                    quote = None;
                }
            }
            None => match byte {
                b'"' | b'\'' => quote = Some(byte),
                b'>' => return Some(cursor + 1),
                _ => {}
            },
        }
        cursor += 1;
    }
    None
}

/// If a comment starts at `index`, return the offset just past it.
fn comment_end(input: &str, index: usize) -> Option<usize> {
    let rest = input.get(index..)?;
    if !rest.starts_with("<!--") {
        return None;
    }
    // Keep conditional comments: they are instructions for old browsers.
    if rest.starts_with("<!--[if") || rest.starts_with("<!--<![endif") {
        return None;
    }
    rest[4..].find("-->").map(|offset| index + 4 + offset + 3)
}

/// If a raw-text element starts at `index`, return the offset just past its
/// closing tag.
fn raw_element_end(input: &str, index: usize) -> Option<usize> {
    let rest = input.get(index..)?;
    // Compare against the source instead of lowercasing a fixed-size prefix:
    // this runs for every `<` in the document, and the old version allocated a
    // `String` per candidate element.
    let name = RAW_ELEMENTS.iter().copied().find(|name| {
        rest.get(1..1 + name.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(name))
            && rest
                .get(1 + name.len()..)
                .is_some_and(|tail| tail.starts_with([' ', '>', '\n', '\t', '\r']))
    })?;
    match find_closing(input, index, name) {
        Some(start) => Some(
            input[start..]
                .find('>')
                .map(|end| start + end + 1)
                .unwrap_or(input.len()),
        ),
        None => Some(input.len()),
    }
}

fn find_closing(input: &str, from: usize, name: &str) -> Option<usize> {
    let mut cursor = from;
    while let Some(offset) = input[cursor..].find("</") {
        let start = cursor + offset;
        let after = &input[start + 2..];
        if after
            .get(..name.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(name))
        {
            return Some(start);
        }
        cursor = start + 2;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_indentation_and_keeps_inline_spacing() {
        let input = "<p>\n  Hello\n  <span>a</span> <span>b</span>\n</p>\n";
        assert_eq!(
            minify_html(input),
            "<p>Hello <span>a</span> <span>b</span></p>"
        );
    }

    #[test]
    fn preserves_pre_and_script_content() {
        let input = "<pre>\n  keep\n    this\n</pre><script>\n  let a = 1;\n</script>";
        assert_eq!(minify_html(input), input);
    }

    #[test]
    fn preserves_raw_elements_with_attributes_and_case() {
        let input = "<textarea class=\"x\">  keep  </textarea><PRE data-x=\"1\">\n a\n</PRE>";
        assert_eq!(minify_html(input), input);
    }

    #[test]
    fn removes_comments_but_keeps_conditional_ones() {
        let input = "<!-- drop me --><p>x</p><!--[if IE]><p>old</p><![endif]-->";
        assert_eq!(
            minify_html(input),
            "<p>x</p><!--[if IE]><p>old</p><![endif]-->"
        );
    }

    #[test]
    fn handles_unicode_and_multibyte_text() {
        let input = "<p>  Şık   ve  hızlı  </p>";
        assert_eq!(minify_html(input), "<p>Şık ve hızlı</p>");
    }

    #[test]
    fn keeps_a_stray_less_than_sign_as_text() {
        let input = "<p>a < b</p>";
        assert_eq!(minify_html(input), "<p>a < b</p>");
    }

    #[test]
    fn does_not_collapse_whitespace_inside_quoted_attributes() {
        let input = "<div title=\"a > b\">x</div><img alt='c > d'>";
        assert_eq!(minify_html(input), input);
        assert_eq!(
            minify_html("<p>\n  <a href=\"/a\" title=\"q > r\">link</a>\n</p>"),
            "<p><a href=\"/a\" title=\"q > r\">link</a></p>"
        );
    }

    #[test]
    fn an_unterminated_quote_leaves_the_tag_as_text() {
        // No closing quote, so there is no reliable tag end: emit the source.
        let input = "<div title=\"oops>text";
        assert_eq!(minify_html(input), input);
    }

    #[test]
    fn minifying_twice_is_minifying_once() {
        for input in [
            "<p>\n  a  <b>b</b>\n</p>",
            "<div title=\"a > b\">\n x \n</div>",
            "<pre>\n  keep\n</pre>",
            "<!-- c --><p>x</p>",
            "<ul>\n <li>a</li>\n <li>b</li>\n</ul>",
            "<span>a</span> <span>b</span>",
            "<p>Şık   ve  hızlı</p>",
            "<a href=\"/x\" title='q > r'>link</a>",
        ] {
            let once = minify_html(input);
            assert_eq!(minify_html(&once), once, "{input:?}");
        }
    }
}
