//! File types as MIME names, the way shared-mime-info names them, so a
//! `.desktop` file's `MimeType=` and `mimeapps.list` mean the same here as on
//! any desktop. Our own table, not the database: it is read once per row in
//! Files, and the image keeps shared-mime-info only as a dependency.
//!
//! The order follows the spec: a known file name, then the extension, then
//! the first bytes, then "is it text".

use std::io::Read;
use std::path::Path;

pub const DIR: &str = "inode/directory";
pub const TEXT: &str = "text/plain";
pub const BINARY: &str = "application/octet-stream";
/// How much of a file [`of_path`] reads to sniff it.
pub const HEAD: usize = 4096;

/// Lower-case extensions, sorted for a binary search.
const EXT: &[(&str, &str)] = &[
    ("3g2", "video/3gpp2"),
    ("3gp", "video/3gpp"),
    ("7z", "application/x-7z-compressed"),
    ("a", "application/x-archive"),
    ("aac", "audio/aac"),
    ("ac3", "audio/ac3"),
    ("adoc", "text/plain"),
    ("aif", "audio/x-aiff"),
    ("aifc", "audio/x-aifc"),
    ("aiff", "audio/x-aiff"),
    ("amr", "audio/AMR"),
    ("ape", "audio/x-ape"),
    ("apk", "application/vnd.android.package-archive"),
    ("arw", "image/x-sony-arw"),
    ("asciidoc", "text/plain"),
    ("asf", "application/vnd.ms-asf"),
    ("asm", "text/x-asm"),
    ("ass", "text/x-ssa"),
    ("au", "audio/basic"),
    ("avi", "video/vnd.avi"),
    ("avif", "image/avif"),
    ("awk", "application/x-awk"),
    ("bash", "application/x-shellscript"),
    ("bat", "application/x-bat"),
    ("bdf", "application/x-font-bdf"),
    ("bib", "text/x-bibtex"),
    ("bin", "application/octet-stream"),
    ("bmp", "image/bmp"),
    ("bz2", "application/x-bzip2"),
    ("c", "text/x-csrc"),
    ("cab", "application/vnd.ms-cab-compressed"),
    ("caf", "audio/x-caf"),
    ("cc", "text/x-c++src"),
    ("cfg", "text/plain"),
    ("cjs", "text/javascript"),
    ("clj", "text/x-clojure"),
    ("cljs", "text/x-clojure"),
    ("cmake", "text/x-cmake"),
    ("cmd", "application/x-bat"),
    ("conf", "text/plain"),
    ("cpio", "application/x-cpio"),
    ("cpp", "text/x-c++src"),
    ("cr2", "image/x-canon-cr2"),
    ("cr3", "image/x-canon-cr3"),
    ("cs", "text/x-csharp"),
    ("css", "text/css"),
    ("csv", "text/csv"),
    ("cur", "image/x-win-bitmap"),
    ("cxx", "text/x-c++src"),
    ("d", "text/x-dsrc"),
    ("dart", "application/vnd.dart"),
    ("db", "application/octet-stream"),
    ("deb", "application/vnd.debian.binary-package"),
    ("desktop", "application/x-desktop"),
    ("dib", "image/bmp"),
    ("diff", "text/x-patch"),
    ("divx", "video/vnd.avi"),
    ("djvu", "image/vnd.djvu"),
    ("dng", "image/x-adobe-dng"),
    ("doc", "application/msword"),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("dts", "audio/vnd.dts"),
    ("el", "text/x-emacs-lisp"),
    ("elf", "application/x-executable"),
    ("elm", "text/x-elm"),
    ("env", "text/plain"),
    ("epub", "application/epub+zip"),
    ("erl", "text/x-erlang"),
    ("ex", "text/x-elixir"),
    ("exe", "application/x-msdownload"),
    ("exr", "image/x-exr"),
    ("exs", "text/x-elixir"),
    ("f4v", "video/mp4"),
    ("fish", "application/x-fishscript"),
    ("flac", "audio/flac"),
    ("flv", "video/x-flv"),
    ("fs", "text/x-fsharp"),
    ("gif", "image/gif"),
    ("go", "text/x-go"),
    ("gql", "text/plain"),
    ("gradle", "text/x-gradle"),
    ("graphql", "text/plain"),
    ("groovy", "text/x-groovy"),
    ("gz", "application/gzip"),
    ("h", "text/x-chdr"),
    ("hdr", "image/vnd.radiance"),
    ("heic", "image/heif"),
    ("heif", "image/heif"),
    ("hh", "text/x-c++hdr"),
    ("hpp", "text/x-c++hdr"),
    ("hrl", "text/x-erlang"),
    ("hs", "text/x-haskell"),
    ("htm", "text/html"),
    ("html", "text/html"),
    ("hxx", "text/x-c++hdr"),
    ("ico", "image/vnd.microsoft.icon"),
    ("img", "application/vnd.efi.img"),
    ("ini", "text/plain"),
    ("iso", "application/vnd.efi.iso"),
    ("it", "audio/x-it"),
    ("j2k", "image/x-jp2-codestream"),
    ("jar", "application/java-archive"),
    ("java", "text/x-java"),
    ("jfif", "image/jpeg"),
    ("jl", "text/julia"),
    ("jp2", "image/jp2"),
    ("jpe", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("jpf", "image/jpx"),
    ("jpg", "image/jpeg"),
    ("jpx", "application/x-jbuilder-project"),
    ("js", "text/javascript"),
    ("json", "application/json"),
    ("jsonl", "application/json"),
    ("jsx", "text/javascript"),
    ("jxl", "image/jxl"),
    ("ksh", "application/x-shellscript"),
    ("kt", "text/x-kotlin"),
    ("kts", "text/x-kotlin"),
    ("less", "text/css"),
    ("lisp", "text/x-common-lisp"),
    ("log", "text/x-log"),
    ("lua", "text/x-lua"),
    ("lz", "application/x-lzip"),
    ("lz4", "application/x-lz4"),
    ("lzma", "application/x-lzma"),
    ("m", "text/x-objcsrc"),
    ("m2ts", "video/mp2t"),
    ("m4a", "audio/mp4"),
    ("m4b", "audio/x-m4b"),
    ("m4v", "video/mp4"),
    ("markdown", "text/markdown"),
    ("md", "text/markdown"),
    ("mid", "audio/midi"),
    ("midi", "audio/midi"),
    ("mjs", "text/javascript"),
    ("mk", "text/x-makefile"),
    ("mka", "audio/x-matroska"),
    ("mkv", "video/x-matroska"),
    ("ml", "text/x-ocaml"),
    ("mli", "text/x-ocaml"),
    ("mm", "text/x-objc++src"),
    ("mobi", "application/x-mobipocket-ebook"),
    ("mod", "audio/x-mod"),
    ("mount", "text/x-systemd-unit"),
    ("mov", "video/quicktime"),
    ("mp3", "audio/mpeg"),
    ("mp4", "video/mp4"),
    ("mpe", "video/mpeg"),
    ("mpeg", "video/mpeg"),
    ("mpg", "video/mpeg"),
    ("mts", "video/mp2t"),
    ("nef", "image/x-nikon-nef"),
    ("nfo", "text/plain"),
    ("nim", "text/x-nim"),
    ("nix", "text/x-nix"),
    ("o", "application/x-object"),
    ("odp", "application/vnd.oasis.opendocument.presentation"),
    ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("oga", "audio/ogg"),
    ("ogg", "audio/ogg"),
    ("ogv", "video/ogg"),
    ("opus", "audio/ogg"),
    ("orf", "image/x-olympus-orf"),
    ("org", "text/org"),
    ("otf", "application/vnd.oasis.opendocument.formula-template"),
    ("pam", "image/x-portable-anymap"),
    ("pas", "text/x-pascal"),
    ("patch", "text/x-patch"),
    ("path", "text/x-systemd-unit"),
    ("pbm", "image/x-portable-bitmap"),
    ("pcf", "application/x-font-pcf"),
    ("pdf", "application/pdf"),
    ("pef", "image/x-pentax-pef"),
    ("pgm", "image/x-portable-graymap"),
    ("php", "application/x-php"),
    ("pl", "application/x-perl"),
    ("pm", "application/x-perl"),
    ("png", "image/png"),
    ("pnm", "image/x-portable-anymap"),
    ("po", "text/x-gettext-translation"),
    ("pot", "application/vnd.ms-powerpoint"),
    ("ppm", "image/x-portable-pixmap"),
    ("ppt", "application/vnd.ms-powerpoint"),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    ("properties", "text/plain"),
    ("proto", "text/x-protobuf"),
    ("ps1", "application/x-powershell"),
    ("psd", "image/vnd.adobe.photoshop"),
    ("py", "text/x-python"),
    ("pyi", "text/x-python3"),
    ("pyw", "text/x-python"),
    ("qcow2", "application/x-qemu-disk"),
    ("qoi", "image/qoi"),
    ("qt", "video/quicktime"),
    ("r", "text/x-r"),
    ("raf", "image/x-fuji-raf"),
    ("rar", "application/vnd.rar"),
    ("rb", "application/x-ruby"),
    ("rkt", "text/x-scheme"),
    ("rm", "application/vnd.rn-realmedia"),
    ("rmvb", "application/vnd.rn-realmedia"),
    ("rpm", "application/x-rpm"),
    ("rs", "text/rust"),
    ("rst", "text/x-rst"),
    ("rtf", "application/rtf"),
    ("rw2", "image/x-panasonic-rw2"),
    ("s", "text/x-asm"),
    ("s3m", "audio/x-s3m"),
    ("sass", "text/x-sass"),
    ("scala", "text/x-scala"),
    ("scm", "text/x-scheme"),
    ("scss", "text/x-scss"),
    ("sed", "text/plain"),
    ("service", "text/x-dbus-service"),
    ("sh", "application/x-shellscript"),
    ("snd", "audio/basic"),
    ("so", "application/x-sharedlib"),
    ("socket", "text/x-systemd-unit"),
    ("spx", "audio/x-speex+ogg"),
    ("sql", "application/sql"),
    ("sqlite", "application/vnd.sqlite3"),
    ("sqlite3", "application/vnd.sqlite3"),
    ("srt", "application/x-subrip"),
    ("srw", "image/x-samsung-srw"),
    ("ssa", "text/x-ssa"),
    ("svelte", "text/html"),
    ("svg", "image/svg+xml"),
    ("svgz", "image/svg+xml-compressed"),
    ("swift", "text/x-swift"),
    ("tar", "application/x-tar"),
    ("target", "text/x-systemd-unit"),
    ("tbz2", "application/x-bzip2-compressed-tar"),
    ("tcl", "text/tcl"),
    ("tex", "text/x-tex"),
    ("text", "text/plain"),
    ("tf", "text/plain"),
    ("tga", "image/x-tga"),
    ("tgz", "application/x-compressed-tar"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("timer", "text/x-systemd-unit"),
    ("toml", "application/toml"),
    ("ts", "text/x-typescript"),
    ("tsv", "text/tab-separated-values"),
    ("tsx", "text/x-typescript"),
    ("ttf", "font/ttf"),
    ("txt", "text/plain"),
    ("txz", "application/x-xz-compressed-tar"),
    ("tzst", "application/x-zstd-compressed-tar"),
    ("vb", "application/x-virtual-boy-rom"),
    ("vim", "text/plain"),
    ("vob", "video/mpeg"),
    ("vtt", "text/vtt"),
    ("vue", "text/html"),
    ("wav", "audio/vnd.wave"),
    ("webm", "video/webm"),
    ("webp", "image/webp"),
    ("wma", "audio/x-ms-wma"),
    ("wmv", "video/x-ms-wmv"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("wv", "audio/x-wavpack"),
    ("xhtml", "application/xhtml+xml"),
    ("xls", "application/vnd.ms-excel"),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("xm", "audio/x-xm"),
    ("xml", "application/xml"),
    ("xz", "application/x-xz"),
    ("y4m", "video/x-yuv4mpeg"),
    ("yaml", "application/yaml"),
    ("yml", "application/yaml"),
    ("z", "application/x-compress"),
    ("zig", "text/x-zig"),
    ("zip", "application/zip"),
    ("zsh", "application/x-shellscript"),
    ("zst", "application/zstd"),
];

/// Whole file names that say what a file is better than its extension.
const NAMES: &[(&str, &str)] = &[
    (".bash_profile", "application/x-shellscript"),
    (".bashrc", "application/x-shellscript"),
    (".gitignore", TEXT),
    (".profile", "application/x-shellscript"),
    (".zshrc", "application/x-shellscript"),
    ("CMakeLists.txt", "text/x-cmake"),
    ("COPYING", "text/x-copying"),
    ("Containerfile", "text/x-dockerfile"),
    ("Dockerfile", "text/x-dockerfile"),
    ("GNUmakefile", "text/x-makefile"),
    ("Gemfile", "application/x-ruby"),
    ("LICENSE", "text/x-copying"),
    ("Makefile", "text/x-makefile"),
    ("PKGBUILD", "application/x-shellscript"),
    ("README", "text/x-readme"),
    ("Rakefile", "application/x-ruby"),
    ("Vagrantfile", "application/x-ruby"),
    ("makefile", "text/x-makefile"),
    ("meson.build", "text/x-meson"),
];

/// Double extensions checked before the last one.
const SUFFIXES: &[(&str, &str)] = &[
    (".tar.bz2", "application/x-bzip2-compressed-tar"),
    (".tar.gz", "application/x-compressed-tar"),
    (".tar.lz4", "application/x-lz4-compressed-tar"),
    (".tar.xz", "application/x-xz-compressed-tar"),
    (".tar.zst", "application/x-zstd-compressed-tar"),
];

/// `sub-class-of` from shared-mime-info, for the types above. Besides these,
/// every `text/*` is a kind of `text/plain` and everything but `inode/*` a
/// kind of `application/octet-stream`.
const PARENTS: &[(&str, &str)] = &[
    ("application/epub+zip", "application/zip"),
    ("application/java-archive", "application/zip"),
    ("application/json", "application/json5"),
    ("application/json5", "text/javascript"),
    ("application/msword", "application/x-ole-storage"),
    ("application/rtf", "text/plain"),
    ("application/sql", "text/plain"),
    ("application/toml", "text/plain"),
    (
        "application/vnd.android.package-archive",
        "application/java-archive",
    ),
    ("application/vnd.dart", "text/plain"),
    ("application/vnd.efi.iso", "application/vnd.efi.img"),
    ("application/vnd.ms-excel", "application/x-ole-storage"),
    ("application/vnd.ms-powerpoint", "application/x-ole-storage"),
    (
        "application/vnd.oasis.opendocument.formula-template",
        "application/zip",
    ),
    (
        "application/vnd.oasis.opendocument.presentation",
        "application/zip",
    ),
    (
        "application/vnd.oasis.opendocument.spreadsheet",
        "application/zip",
    ),
    ("application/vnd.oasis.opendocument.text", "application/zip"),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/zip",
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/zip",
    ),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/zip",
    ),
    ("application/vnd.sqlite3", "application/octet-stream"),
    ("application/x-awk", "application/x-executable"),
    ("application/x-awk", "text/plain"),
    ("application/x-bat", "text/plain"),
    ("application/x-bzip2-compressed-tar", "application/x-bzip2"),
    ("application/x-compressed-tar", "application/gzip"),
    ("application/x-desktop", "text/plain"),
    ("application/x-fishscript", "application/x-executable"),
    ("application/x-fishscript", "text/plain"),
    ("application/x-mobipocket-ebook", "application/vnd.palm"),
    ("application/x-msdownload", "application/x-executable"),
    ("application/x-perl", "application/x-executable"),
    ("application/x-perl", "text/plain"),
    ("application/x-php", "text/plain"),
    ("application/x-powershell", "text/plain"),
    ("application/x-ruby", "application/x-executable"),
    ("application/x-ruby", "text/plain"),
    ("application/x-shellscript", "application/x-executable"),
    ("application/x-shellscript", "text/plain"),
    ("application/x-subrip", "text/plain"),
    ("application/x-tiled-tsx", "application/xml"),
    ("application/x-xz-compressed-tar", "application/x-xz"),
    ("application/x-zstd-compressed-tar", "application/zstd"),
    ("application/xhtml+xml", "application/xml"),
    ("application/xml", "text/plain"),
    ("application/yaml", "text/plain"),
    ("audio/ogg", "application/ogg"),
    ("audio/vnd.wave", "application/x-riff"),
    ("audio/x-aifc", "application/x-iff"),
    ("audio/x-aiff", "application/x-iff"),
    ("audio/x-m4b", "audio/mp4"),
    ("audio/x-matroska", "application/x-matroska"),
    ("audio/x-ms-wma", "application/vnd.ms-asf"),
    ("audio/x-speex+ogg", "audio/ogg"),
    ("image/svg+xml-compressed", "application/gzip"),
    ("image/svg+xml", "application/xml"),
    ("image/x-adobe-dng", "image/tiff"),
    ("image/x-adobe-dng", "image/x-dcraw"),
    ("image/x-canon-cr2", "image/tiff"),
    ("image/x-canon-cr2", "image/x-dcraw"),
    ("image/x-canon-cr3", "image/x-dcraw"),
    ("image/x-fuji-raf", "image/x-dcraw"),
    ("image/x-nikon-nef", "image/tiff"),
    ("image/x-nikon-nef", "image/x-dcraw"),
    ("image/x-olympus-orf", "image/x-dcraw"),
    ("image/x-panasonic-rw2", "image/x-dcraw"),
    ("image/x-pentax-pef", "image/tiff"),
    ("image/x-pentax-pef", "image/x-dcraw"),
    ("image/x-portable-bitmap", "image/x-portable-anymap"),
    ("image/x-portable-graymap", "image/x-portable-anymap"),
    ("image/x-portable-pixmap", "image/x-portable-anymap"),
    ("image/x-samsung-srw", "image/x-dcraw"),
    ("image/x-sony-arw", "image/tiff"),
    ("image/x-sony-arw", "image/x-dcraw"),
    ("text/css", "text/plain"),
    ("text/csv", "text/plain"),
    ("text/html", "text/plain"),
    ("text/javascript", "application/x-executable"),
    ("text/javascript", "text/plain"),
    ("text/julia", "text/plain"),
    ("text/markdown", "text/plain"),
    ("text/org", "text/plain"),
    ("text/rust", "text/plain"),
    ("text/tab-separated-values", "text/plain"),
    ("text/tcl", "text/plain"),
    ("text/vnd.trolltech.linguist", "application/xml"),
    ("text/vtt", "text/plain"),
    ("text/x-asm", "text/plain"),
    ("text/x-bibtex", "text/plain"),
    ("text/x-c++hdr", "text/x-chdr"),
    ("text/x-c++src", "text/x-csrc"),
    ("text/x-chdr", "text/x-csrc"),
    ("text/x-clojure", "text/plain"),
    ("text/x-cmake", "text/plain"),
    ("text/x-common-lisp", "text/plain"),
    ("text/x-copying", "text/plain"),
    ("text/x-csharp", "text/x-csrc"),
    ("text/x-csrc", "text/plain"),
    ("text/x-dbus-service", "text/plain"),
    ("text/x-dsrc", "text/x-csrc"),
    ("text/x-elixir", "text/plain"),
    ("text/x-elm", "text/plain"),
    ("text/x-emacs-lisp", "text/plain"),
    ("text/x-erlang", "text/plain"),
    ("text/x-fsharp", "text/plain"),
    ("text/x-gettext-translation", "text/plain"),
    ("text/x-go", "text/plain"),
    ("text/x-gradle", "text/x-groovy"),
    ("text/x-groovy", "text/x-csrc"),
    ("text/x-haskell", "text/plain"),
    ("text/x-java", "text/x-csrc"),
    ("text/x-kotlin", "text/plain"),
    ("text/x-log", "text/plain"),
    ("text/x-lua", "application/x-executable"),
    ("text/x-lua", "text/plain"),
    ("text/x-makefile", "text/plain"),
    ("text/x-meson", "text/plain"),
    ("text/x-nim", "text/plain"),
    ("text/x-nix", "text/plain"),
    ("text/x-objc++src", "text/x-c++src"),
    ("text/x-objc++src", "text/x-objcsrc"),
    ("text/x-objcsrc", "text/x-csrc"),
    ("text/x-ocaml", "text/plain"),
    ("text/x-pascal", "text/plain"),
    ("text/x-patch", "text/plain"),
    ("text/x-protobuf", "text/plain"),
    ("text/x-python", "application/x-executable"),
    ("text/x-python", "text/plain"),
    ("text/x-python3", "text/x-python"),
    ("text/x-r", "text/plain"),
    ("text/x-readme", "text/plain"),
    ("text/x-rst", "text/plain"),
    ("text/x-sass", "text/plain"),
    ("text/x-scala", "text/plain"),
    ("text/x-scheme", "text/plain"),
    ("text/x-scss", "text/plain"),
    ("text/x-ssa", "text/plain"),
    ("text/x-swift", "text/plain"),
    ("text/x-systemd-unit", "text/plain"),
    ("text/x-tex", "text/plain"),
    ("text/x-typescript", "text/javascript"),
    ("text/x-zig", "text/plain"),
    ("video/3gpp", "video/mp4"),
    ("video/ogg", "application/ogg"),
    ("video/x-matroska", "application/x-matroska"),
    ("video/x-ms-wmv", "application/vnd.ms-asf"),
];

fn by_ext(ext: &str) -> Option<&'static str> {
    let lower;
    let ext = if ext.bytes().any(|b| b.is_ascii_uppercase()) {
        // `*.C` is C++ in the database: the one case that matters here.
        if ext == "C" {
            return Some("text/x-c++src");
        }
        lower = ext.to_ascii_lowercase();
        lower.as_str()
    } else {
        ext
    };
    EXT.binary_search_by(|(e, _)| (*e).cmp(ext))
        .ok()
        .map(|i| EXT[i].1)
}

/// The type a file name says, if it says one. Cheap: no allocation for a
/// lower-case name.
pub fn of_name(name: &str) -> Option<&'static str> {
    if let Some(&(_, m)) = NAMES.iter().find(|(n, _)| *n == name) {
        return Some(m);
    }
    if name.len() > 8 && name.is_char_boundary(name.len() - 8) {
        let tail = name[name.len() - 8..].to_ascii_lowercase();
        if let Some(&(_, m)) = SUFFIXES.iter().find(|(s, _)| tail.ends_with(s)) {
            return Some(m);
        }
    }
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    by_ext(ext)
}

fn riff(head: &[u8], kind: &[u8; 4]) -> bool {
    head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == kind
}

/// The type the first bytes say. `None` when they say nothing.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    let at = |off: usize, magic: &[u8]| head.get(off..off + magic.len()) == Some(magic);
    let m = if at(0, b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if at(0, b"\xff\xd8\xff") {
        "image/jpeg"
    } else if at(0, b"GIF87a") || at(0, b"GIF89a") {
        "image/gif"
    } else if riff(head, b"WEBP") {
        "image/webp"
    } else if riff(head, b"WAVE") {
        "audio/vnd.wave"
    } else if riff(head, b"AVI ") {
        "video/vnd.avi"
    } else if at(0, b"BM") && head.len() >= 14 && at(6, b"\0\0\0\0") {
        "image/bmp"
    } else if at(0, b"II*\0") || at(0, b"MM\0*") {
        "image/tiff"
    } else if at(0, b"\xff\x0a") || at(0, b"\0\0\0\x0cJXL \r\n\x87\n") {
        "image/jxl"
    } else if at(4, b"ftyp") {
        match head.get(8..12) {
            Some(b"heic" | b"heix" | b"mif1" | b"msf1" | b"heim" | b"heis") => "image/heif",
            Some(b"avif" | b"avis") => "image/avif",
            Some(b"qt  ") => "video/quicktime",
            Some(b"M4A " | b"M4B ") => "audio/mp4",
            Some(b"3gp4" | b"3gp5" | b"3gp6" | b"3g2a") => "video/3gpp",
            _ => "video/mp4",
        }
    } else if at(0, b"\x1a\x45\xdf\xa3") {
        let n = head.len().min(64);
        if head[..n].windows(4).any(|w| w == b"webm") {
            "video/webm"
        } else {
            "video/x-matroska"
        }
    } else if at(0, b"OggS") {
        let n = head.len().min(64);
        if head[..n].windows(6).any(|w| w == b"theora") {
            "video/ogg"
        } else {
            "audio/ogg"
        }
    } else if at(0, b"fLaC") {
        "audio/flac"
    } else if at(0, b"ID3") {
        "audio/mpeg"
    } else if at(0, b"FLV\x01") {
        "video/x-flv"
    } else if at(0, b"%PDF-") {
        "application/pdf"
    } else if at(0, b"PK\x03\x04") {
        "application/zip"
    } else if at(0, b"\x1f\x8b") {
        "application/gzip"
    } else if at(0, b"\xfd7zXZ\0") {
        "application/x-xz"
    } else if at(0, b"\x28\xb5\x2f\xfd") {
        "application/zstd"
    } else if at(0, b"BZh") {
        "application/x-bzip2"
    } else if at(0, b"7z\xbc\xaf\x27\x1c") {
        "application/x-7z-compressed"
    } else if at(0, b"Rar!\x1a\x07") {
        "application/vnd.rar"
    } else if at(257, b"ustar") {
        "application/x-tar"
    } else if at(0, b"\x7fELF") {
        "application/x-executable"
    } else if at(0, b"SQLite format 3\0") {
        "application/vnd.sqlite3"
    } else if at(0, b"\x47") && at(188, b"\x47") {
        "video/mp2t"
    } else if at(0, b"#!") {
        let line = head.split(|&b| b == b'\n').next().unwrap_or(&[]);
        let line = std::str::from_utf8(&line[2..]).unwrap_or("");
        let mut words = line
            .split_whitespace()
            .map(|w| w.rsplit('/').next().unwrap_or(w));
        let mut prog = words.next().unwrap_or("");
        if prog == "env" {
            prog = words.find(|w| !w.starts_with('-')).unwrap_or("");
        }
        match prog {
            p if p.starts_with("python") => "text/x-python",
            p if p.starts_with("perl") => "application/x-perl",
            p if p.starts_with("ruby") => "application/x-ruby",
            p if p.starts_with("lua") => "text/x-lua",
            "node" | "deno" | "bun" => "text/javascript",
            "fish" => "application/x-fishscript",
            "awk" | "gawk" | "mawk" => "application/x-awk",
            _ => "application/x-shellscript",
        }
    } else if at(0, b"<?xml") || at(0, b"<svg") {
        let n = head.len().min(512);
        if head[..n].windows(4).any(|w| w == b"<svg") {
            "image/svg+xml"
        } else {
            "application/xml"
        }
    } else {
        return None;
    };
    Some(m)
}

/// Text: no NUL and valid UTF-8, allowing a character cut at the end.
pub fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && head.len() - e.valid_up_to() < 4,
    }
}

/// What `path` is, reading at most [`HEAD`] bytes of it. A folder is
/// [`DIR`]; anything else that is not a plain file is [`BINARY`]. An empty
/// file is text, so it opens in an editor.
pub fn of_path(path: &Path) -> std::io::Result<&'static str> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        return Ok(DIR);
    }
    if !meta.is_file() {
        return Ok(BINARY);
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let by_name = of_name(name);
    // `.ts` is TypeScript or an MPEG stream: the bytes decide.
    if let Some(m) = by_name.filter(|_| !name.to_ascii_lowercase().ends_with(".ts")) {
        return Ok(m);
    }
    let mut head = Vec::with_capacity(HEAD);
    std::fs::File::open(path)?
        .take(HEAD as u64)
        .read_to_end(&mut head)?;
    Ok(sniff(&head)
        .or(by_name)
        .unwrap_or(if looks_like_text(&head) { TEXT } else { BINARY }))
}

/// The direct parents of `mime`, the implicit ones last.
pub fn parents(mime: &str) -> impl Iterator<Item = &'static str> + '_ {
    let listed = PARENTS
        .iter()
        .filter(move |(c, _)| *c == mime)
        .map(|(_, p)| *p);
    let text = (mime.starts_with("text/") && mime != TEXT).then_some(TEXT);
    let any = (!mime.starts_with("inode/") && mime != BINARY).then_some(BINARY);
    listed.chain(text).chain(any)
}

/// `mime` and every type it is a kind of, nearest first, each once.
pub fn ancestry(mime: &str) -> Vec<String> {
    let mut out = vec![mime.to_string()];
    let mut i = 0;
    while i < out.len() && out.len() < 32 {
        let next: Vec<&str> = parents(&out[i]).collect();
        for p in next {
            if !out.iter().any(|o| o == p) {
                out.push(p.to_string());
            }
        }
        i += 1;
    }
    // The catch-all goes last whatever path reached it.
    if let Some(at) = out.iter().position(|m| m == BINARY) {
        let b = out.remove(at);
        out.push(b);
    }
    out
}

/// Whether a `MimeType=` entry, which may be `image/*`, covers `mime`.
pub fn matches(pattern: &str, mime: &str) -> bool {
    match pattern.strip_suffix("/*") {
        Some(major) => mime.split('/').next() == Some(major),
        None => pattern == mime,
    }
}

/// `mime` is `parent` or a kind of it.
pub fn is_a(mime: &str, parent: &str) -> bool {
    ancestry(mime).iter().any(|m| matches(parent, m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_are_sorted_and_lower_case() {
        assert!(
            EXT.windows(2).all(|w| w[0].0 < w[1].0),
            "EXT must stay sorted"
        );
        assert!(
            EXT.iter()
                .all(|(e, _)| !e.bytes().any(|b| b.is_ascii_uppercase()))
        );
    }

    #[test]
    fn names_then_double_extensions_then_the_last_one() {
        assert_eq!(of_name("Makefile"), Some("text/x-makefile"));
        assert_eq!(
            of_name("backup.TAR.GZ"),
            Some("application/x-compressed-tar")
        );
        assert_eq!(of_name("main.rs"), Some("text/rust"));
        assert_eq!(of_name("Photo.JPG"), Some("image/jpeg"));
        assert_eq!(of_name("a.c"), Some("text/x-csrc"));
        assert_eq!(of_name("a.C"), Some("text/x-c++src"));
        assert_eq!(of_name(".hidden"), None);
        assert_eq!(of_name("noext"), None);
        assert_eq!(of_name("x."), None);
    }

    #[test]
    fn magic_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff(b"\0\0\0\x18ftypheic...."), Some("image/heif"));
        assert_eq!(sniff(b"\0\0\0\x18ftypisom...."), Some("video/mp4"));
        assert_eq!(
            sniff(b"\x1a\x45\xdf\xa3\x01\0\0\0\0\0\0\x1f\x42\x86\x81\x01\x42\x82\x84webm"),
            Some("video/webm")
        );
        assert_eq!(
            sniff(b"#!/usr/bin/env python3\nprint()"),
            Some("text/x-python")
        );
        assert_eq!(sniff(b"#!/bin/sh\necho"), Some("application/x-shellscript"));
        assert_eq!(sniff(b"#! /usr/bin/python3 -u\n"), Some("text/x-python"));
        assert_eq!(
            sniff(b"#!/usr/bin/env -S node --x\n"),
            Some("text/javascript")
        );
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><svg>"),
            Some("image/svg+xml")
        );
        assert_eq!(sniff(b"hello"), None);
    }

    #[test]
    fn text_is_utf8_without_nul_even_when_cut_mid_character() {
        assert!(looks_like_text(b"plain"));
        assert!(looks_like_text("caf\u{e9}".as_bytes()));
        assert!(looks_like_text(&"caf\u{e9}".as_bytes()[..4]));
        assert!(!looks_like_text(b"a\0b"));
        assert!(!looks_like_text(b"\xff\xfe\x00"));
    }

    #[test]
    fn paths_read_the_bytes_when_the_name_says_nothing() {
        let d = std::env::temp_dir().join(format!("gh0st-mime-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let w = |n: &str, b: &[u8]| {
            std::fs::write(d.join(n), b).unwrap();
            of_path(&d.join(n)).unwrap()
        };
        assert_eq!(w("notes", b"just words\n"), TEXT);
        assert_eq!(w("empty", b""), TEXT);
        assert_eq!(w("blob", b"\x01\x02\0\x03"), BINARY);
        assert_eq!(w("run", b"#!/bin/bash\n"), "application/x-shellscript");
        assert_eq!(w("app.ts", b"const a: number = 1;\n"), "text/x-typescript");
        let mut ts = vec![0u8; 400];
        ts[0] = 0x47;
        ts[188] = 0x47;
        assert_eq!(w("clip.ts", &ts), "video/mp2t");
        assert_eq!(of_path(&d).unwrap(), DIR);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn ancestry_walks_to_plain_text_and_ends_at_octet_stream() {
        assert_eq!(ancestry("text/rust"), ["text/rust", TEXT, BINARY]);
        let json = ancestry("application/json");
        assert!(json.iter().any(|m| m == TEXT), "{json:?}");
        assert_eq!(json.last().map(String::as_str), Some(BINARY));
        assert!(is_a("image/svg+xml", "application/xml"));
        assert!(is_a("image/svg+xml", "image/*"));
        assert!(is_a("application/x-shellscript", TEXT));
        assert!(!is_a("image/png", TEXT));
        assert_eq!(ancestry(DIR), [DIR]);
    }
}
