use crate::parse::Error;
use std::fmt::Display;

/// A list of parsers that parsing can fail on. This is used for pretty-printing
/// errors
#[derive(PartialEq, Debug, Clone, Copy)]
pub(crate) enum ParseNode {
    SectionHeader,
    ConfigName,
}

impl Display for ParseNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SectionHeader => write!(f, "section header"),
            Self::ConfigName => write!(f, "config name"),
        }
    }
}

impl Error {
    /// The one-indexed line number where the error occurred. This is determined
    /// by the number of newlines that were successfully parsed.
    #[must_use]
    pub const fn line_number(&self) -> usize {
        self.line_number + 1
    }

    /// The remaining data that was left unparsed.
    #[must_use]
    pub fn remaining_data(&self) -> &[u8] {
        &self.parsed_until
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data_size = self.parsed_until.len();
        let data = std::str::from_utf8(&self.parsed_until);
        write!(
            f,
            "Got an unexpected token on line {} while trying to parse a {}: ",
            self.line_number + 1,
            self.last_attempted_parser,
        )?;

        match (data, data_size) {
            (Ok(data), _) if data_size > 10 => {
                write!(f, "'{}' ... ({} characters omitted)", &data[..10], data_size - 10)
            }
            (Ok(data), _) => write!(f, "'{}'", data),
            (Err(_), _) if data_size > 10 => write!(
                f,
                "'{:02x?}' ... ({} characters omitted)",
                &self.parsed_until[..10],
                data_size - 10
            ),
            (Err(_), _) => write!(f, "'{:02x?}'", self.parsed_until),
        }
    }
}

impl std::error::Error for Error {}
