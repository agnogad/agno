pub const BASE: &str = include_str!("../theme/base.html");
pub const PAGE: &str = include_str!("../theme/page.html");
pub const SECTION: &str = include_str!("../theme/section.html");
pub const INDEX: &str = include_str!("../theme/index.html");
pub const NOT_FOUND: &str = include_str!("../theme/404.html");
pub const STYLE: &str = include_str!("../theme/style.css");

pub const TEMPLATES: &[(&str, &str)] = &[
    ("base.html", BASE),
    ("page.html", PAGE),
    ("section.html", SECTION),
    ("index.html", INDEX),
    ("404.html", NOT_FOUND),
];
