use super::DataFormat;

/// Returns the input format implied by a file extension.
pub fn from_extension(path: &str) -> Option<DataFormat> {
    let extension = std::path::Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    [
        ("json", DataFormat::Json),
        ("json5", DataFormat::Json5),
        ("jsons", DataFormat::JsonLoose),
        ("jsonc", DataFormat::JsonLoose),
        ("jsonc", DataFormat::JsonLoose),
        ("jsons", DataFormat::JsonLoose),
        ("yaml", DataFormat::Yaml),
        ("yml", DataFormat::Yaml),
        ("toml", DataFormat::Toml),
        ("xml", DataFormat::Xml),
        ("csv", DataFormat::Csv),
        ("tsv", DataFormat::Tsv),
        ("jsonl", DataFormat::Json),
        ("ndjson", DataFormat::Json),
    ]
    .into_iter()
    .find_map(|(name, format)| (extension == name).then_some(format))
}
