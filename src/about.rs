pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NAME_ORIGIN: &str = "Fero (FEH-roh) is Latin for “to carry, bring, bear.”";
pub const CORE_CREDIT: &str = "Built with Rust, GPUI, libtorrent 2.1.2, and SQLite. The desktop text field is adapted from GPUI’s single-line input example.";
pub const SOLAR_CREDIT: &str = "Solar Icons by 480 Design · CC BY 4.0. Fero applies theme colors to the icons; the SVG artwork is unchanged.";
pub const SOLAR_SOURCE: &str = "https://www.npmjs.com/package/@solar-icons/static/v/2.7.0";
pub const SOLAR_LICENSE: &str = "https://creativecommons.org/licenses/by/4.0/";

pub const NOTICES: &[(&str, &str)] = &[
    ("Fero · MIT", include_str!("../LICENSE")),
    (
        "DM Sans · SIL Open Font License 1.1",
        include_str!("../design/fonts/DM-Sans-OFL.txt"),
    ),
    (
        "Instrument Sans · SIL Open Font License 1.1",
        include_str!("../design/fonts/Instrument-Sans-OFL.txt"),
    ),
    (
        "GPUI by Zed Industries · Apache 2.0",
        include_str!("GPUI-LICENSE-APACHE"),
    ),
    (
        "libtorrent 2.1.2 by Arvid Norberg and contributors · BSD 3-Clause and other notices",
        include_str!("../docs/licenses/libtorrent-2.1.2.txt"),
    ),
    (
        "CXX · MIT (also available under Apache 2.0)",
        include_str!("../docs/licenses/CXX-MIT.txt"),
    ),
    (
        "rusqlite · MIT",
        include_str!("../docs/licenses/rusqlite-MIT.txt"),
    ),
    (
        "Boost · Boost Software License 1.0",
        include_str!("../docs/licenses/Boost-1.0.txt"),
    ),
    (
        "OpenSSL 3 · Apache 2.0",
        include_str!("../docs/licenses/OpenSSL-3.txt"),
    ),
    (
        "c-ares · MIT",
        include_str!("../docs/licenses/c-ares-1.34.8.txt"),
    ),
    (
        "rustls · Apache 2.0, ISC, or MIT",
        include_str!("../docs/licenses/rustls-MIT.txt"),
    ),
];
