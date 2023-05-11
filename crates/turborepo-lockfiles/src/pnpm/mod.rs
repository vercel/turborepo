mod data;
mod de;
mod dep_path;
mod ser;

#[derive(Debug, PartialEq, Eq, Clone)]
struct LockfileVersion {
    version: String,
    format: VersionFormat,
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum VersionFormat {
    String,
    Float,
}
