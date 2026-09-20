use thiserror::Error;

use crate::data::DataFormat;

#[derive(Error, Debug)]
pub enum DataError {
    #[error("Unknown data error")]
    Unknown,

    #[error("Data format '{0}' is not supported for input")]
    WrongInputDataFormat(DataFormat),

    #[error("Data format '{0}' is not supported for input")]
    WrongOutputDataFormat(DataFormat),

    #[error("parse error: {message} at line {line}, column {col}")]
    ParseError {
        path: String,
        line: u32,
        col: u32,
        message: String,
    },

    #[error("IO error: {0}")]
    IOError(std::io::Error),

    #[error("EOF")]
    EOF,

    #[error("Cannot index object with type {index_type}")]
    UnexpectedObjectKeyType { index_type: &'static str },

    #[error("Cannot index array with type {index_type}")]
    UnexpectedArrayIndexType { index_type: &'static str },

    #[error("Out of bounds negative array index")]
    OutOfBoundsNegativeArrayIndex,

    #[error("Unable to serialize value type {value_type}")]
    UnableToSerializeValueType { value_type: &'static str },
}
