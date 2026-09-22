pub mod cbor;
mod document;
pub mod env;
pub mod extension;
pub mod jq;
pub mod json;
pub mod raw;
pub mod sv;
pub mod toml;
pub mod xml;
pub mod yaml;

use std::{fmt, str::FromStr};

pub use crate::data::core::{
    DataError, ParseOutput, Parser, PathItem, Serializer, StreamItem, Value,
};
use crate::{
    io::{Input, Output},
    render,
};
use json::{JsonParserOptions, JsonSerializerOptions, KeywordPreset, LooseLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataFormat {
    Raw,      // Line-by-line
    RawSlurp, // Whole input as one string

    #[default]
    Json,

    Json5,
    JsonLoose,
    PyLit, // Python literal
    Csv,
    Csvh,
    Tsv,
    Tsvh,

    Yaml,
    Toml,
    Xml,

    // Shell env format
    Env,       // KEY=VALUE
    ExportEnv, // export KEY=VALUE

    CBOR,

    JQ, // jq ast
}

impl DataFormat {
    const fn as_str(&self) -> &'static str {
        use DataFormat::{
            CBOR, Csv, Csvh, Env, ExportEnv, JQ, Json, Json5, JsonLoose, PyLit, Raw, RawSlurp,
            Toml, Tsv, Tsvh, Xml, Yaml,
        };
        match self {
            Raw => "raw",
            RawSlurp => "rawslurp",
            Json => "json",

            Json5 => "json5",
            JsonLoose => "j",
            PyLit => "pylit",
            Csv => "csv",
            Csvh => "csvh",
            Tsv => "tsv",
            Tsvh => "tsvh",

            Yaml => "yaml",
            Toml => "toml",
            Xml => "xml",

            Env => "env",
            ExportEnv => "exportenv",

            CBOR => "cbor",

            JQ => "jq",
        }
    }
}

impl FromStr for DataFormat {
    type Err = ();

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        use DataFormat::{
            CBOR, Csv, Csvh, Env, ExportEnv, JQ, Json, Json5, JsonLoose, PyLit, Raw, RawSlurp,
            Toml, Tsv, Tsvh, Xml, Yaml,
        };
        let cano: String = name
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();

        Ok(match cano.as_str() {
            "raw" => Raw,
            "rawslurp" => RawSlurp,

            "" | "json" => Json,

            "json5" => Json5,
            "j" | "jsonloose" => JsonLoose,
            "py" | "pylit" => PyLit,
            "csv" => Csv,
            "csvh" => Csvh,
            "tsv" => Tsv,
            "tsvh" => Tsvh,

            "yaml" | "yml" => Yaml,
            "toml" => Toml,
            "xml" => Xml,

            "env" => Env,
            "exportenv" => ExportEnv,

            "cbor" => CBOR,

            "jq" => JQ,
            _ => return Err(()),
        })
    }
}

impl fmt::Display for DataFormat {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

pub enum AnyParser<'a> {
    Xml(xml::XmlParser<'a>),
    Yaml(yaml::YamlParser<'a>),
    Toml(toml::TomlParser<'a>),
    Raw(raw::RawParser<'a>),
    RawSlurp(raw::RawSlurpParser<'a>),
    Sv(sv::SvParser<'a>),
    Env(env::EnvParser<'a>),
    Json(json::JsonParser<'a>),
    Cbor(cbor::CborParser<'a>),
    Jq(jq::JqParser),
}

impl<'a> AnyParser<'a> {
    pub fn new(format: DataFormat, input: Input<'a>) -> Result<Self, DataError> {
        Ok(match format {
            DataFormat::Xml => Self::Xml(xml::XmlParser::new(input)),
            DataFormat::Yaml => Self::Yaml(yaml::YamlParser::new(input)),
            DataFormat::Toml => Self::Toml(toml::TomlParser::new(input)),
            DataFormat::Raw => Self::Raw(raw::RawParser::new(input)),
            DataFormat::RawSlurp => Self::RawSlurp(raw::RawSlurpParser::new(input)),
            DataFormat::Csv | DataFormat::Csvh | DataFormat::Tsv | DataFormat::Tsvh => {
                Self::Sv(sv::SvParser::new(input, format))
            }
            DataFormat::Env | DataFormat::ExportEnv => Self::Env(env::EnvParser::new(input)),
            DataFormat::Json | DataFormat::Json5 | DataFormat::JsonLoose | DataFormat::PyLit => {
                let (loose_level, keywords) = match format {
                    DataFormat::Json => (LooseLevel::Strict, KeywordPreset::JSON),
                    DataFormat::Json5 => (LooseLevel::Json5, KeywordPreset::JSON5),
                    DataFormat::JsonLoose | DataFormat::PyLit => (
                        LooseLevel::Json5,
                        if matches!(format, DataFormat::PyLit) {
                            KeywordPreset::PY_LIT
                        } else {
                            KeywordPreset::JSON
                        },
                    ),
                    _ => (LooseLevel::Loose, KeywordPreset::LOOSE),
                };
                Self::Json(json::JsonParser::new(
                    input,
                    JsonParserOptions {
                        loose_level,
                        keywords,
                    },
                ))
            }
            DataFormat::CBOR => Self::Cbor(cbor::CborParser::new(input)),
            DataFormat::JQ => Self::Jq(jq::JqParser::new(input)),
        })
    }
}

impl Iterator for AnyParser<'_> {
    type Item = ParseOutput;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Xml(x) => x.next(),
            Self::Yaml(x) => x.next(),
            Self::Toml(x) => x.next(),
            Self::Raw(x) => x.next(),
            Self::RawSlurp(x) => x.next(),
            Self::Sv(x) => x.next(),
            Self::Env(x) => x.next(),
            Self::Json(x) => x.next(),
            Self::Cbor(x) => x.next(),
            Self::Jq(x) => x.next(),
        }
    }
}

pub enum AnySerializer {
    Xml(xml::XmlSerializer),
    Yaml(yaml::YamlSerializer),
    Toml(toml::TomlSerializer),
    Raw(raw::RawSerializer),
    Sv(sv::SvSerializer),
    Env(env::EnvSerializer),
    Json(json::JsonSerializer),
    Cbor(cbor::CborSerializer),
    Jq(jq::JqSerializer),
}
impl AnySerializer {
    pub fn from_format(
        format: DataFormat,
        output: Output,
        options: render::Options,
    ) -> Result<Self, DataError> {
        Ok(match format {
            DataFormat::Xml => Self::Xml(xml::XmlSerializer::new(output, options)),
            DataFormat::Yaml => Self::Yaml(yaml::YamlSerializer::new(output, options)),
            DataFormat::Toml => Self::Toml(toml::TomlSerializer::new(output, options)),
            DataFormat::Raw | DataFormat::RawSlurp => {
                Self::Raw(raw::RawSerializer::new(output, options))
            }
            DataFormat::Csv | DataFormat::Csvh | DataFormat::Tsv | DataFormat::Tsvh => {
                Self::Sv(sv::SvSerializer::new(output, options, format))
            }
            DataFormat::Env | DataFormat::ExportEnv => {
                Self::Env(env::EnvSerializer::new(output, options, format))
            }
            DataFormat::Json | DataFormat::Json5 | DataFormat::JsonLoose | DataFormat::PyLit => {
                let keywords = match format {
                    DataFormat::Json5 => KeywordPreset::JSON5,
                    DataFormat::PyLit => KeywordPreset::PY_LIT,
                    DataFormat::JsonLoose => KeywordPreset::LOOSE,
                    _ => KeywordPreset::JSON,
                };
                Self::Json(json::JsonSerializer::with_options(
                    output,
                    options,
                    JsonSerializerOptions { keywords },
                ))
            }
            DataFormat::CBOR => Self::Cbor(cbor::CborSerializer::new(output)),
            DataFormat::JQ => Self::Jq(jq::JqSerializer::new(output, options)),
        })
    }
    pub fn put(&mut self, value: Value) -> Result<(), DataError> {
        match self {
            Self::Xml(x) => x.put(value),
            Self::Yaml(x) => x.put(value),
            Self::Toml(x) => x.put(value),
            Self::Raw(x) => x.put(value),
            Self::Sv(x) => x.put(value),
            Self::Env(x) => x.put(value),
            Self::Json(x) => x.put(value),
            Self::Cbor(x) => x.put(value),
            Self::Jq(x) => x.put(value),
        }
    }
    pub fn put_all<I: Iterator<Item = Result<Value, DataError>>>(
        &mut self,
        iter: I,
    ) -> Result<(), DataError> {
        for v in iter {
            self.put(v?)?;
        }
        Ok(())
    }
    pub fn finish(self) -> std::io::Result<()> {
        match self {
            Self::Xml(x) => x.finish(),
            Self::Yaml(x) => x.finish(),
            Self::Toml(x) => x.finish(),
            Self::Raw(x) => x.finish(),
            Self::Sv(x) => x.finish(),
            Self::Env(x) => x.finish(),
            Self::Json(x) => x.finish(),
            Self::Cbor(x) => x.finish(),
            Self::Jq(x) => x.finish(),
        }
    }
}
