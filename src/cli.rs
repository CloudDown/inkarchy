use std::path::PathBuf;

/// What `main` should do with the command line.
#[derive(Debug, PartialEq, Eq)]
pub enum CliRequest {
    Help,
    Version,
    DataDir,
    Theme,
    Run(Vec<PathBuf>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    Unknown(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(arg) => write!(f, "unknown argument `{arg}`"),
        }
    }
}

pub fn parse_args<I, S>(args: I) -> Result<CliRequest, CliError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut files = Vec::new();
    let mut query = None;
    let mut positional = false;
    for arg in args {
        let arg = arg.as_ref();
        if positional {
            files.push(PathBuf::from(arg));
            continue;
        }
        match arg {
            "--" => positional = true,
            "--help" | "-h" => query = Some(CliRequest::Help),
            "--version" | "-V" => {
                if !matches!(query, Some(CliRequest::Help)) {
                    query = Some(CliRequest::Version);
                }
            }
            "--data-dir" => {
                if query.is_none() {
                    query = Some(CliRequest::DataDir);
                }
            }
            "--theme" => {
                if query.is_none() {
                    query = Some(CliRequest::Theme);
                }
            }
            other if other.starts_with('-') => {
                return Err(CliError::Unknown(other.to_string()));
            }
            other => files.push(PathBuf::from(other)),
        }
    }
    if let Some(query) = query {
        return Ok(query);
    }
    Ok(CliRequest::Run(files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_open_the_shelf() {
        assert_eq!(
            parse_args(Vec::<&str>::new()).unwrap(),
            CliRequest::Run(vec![])
        );
    }

    #[test]
    fn flags_and_files() {
        assert_eq!(parse_args(["--help"]).unwrap(), CliRequest::Help);
        assert_eq!(parse_args(["-h", "notes.pdf"]).unwrap(), CliRequest::Help);
        assert_eq!(parse_args(["--version"]).unwrap(), CliRequest::Version);
        assert_eq!(parse_args(["-V"]).unwrap(), CliRequest::Version);
        assert_eq!(parse_args(["--data-dir"]).unwrap(), CliRequest::DataDir);
        assert_eq!(parse_args(["--theme"]).unwrap(), CliRequest::Theme);
        assert_eq!(
            parse_args(["lecture.pdf", "figure.png"]).unwrap(),
            CliRequest::Run(vec![
                PathBuf::from("lecture.pdf"),
                PathBuf::from("figure.png")
            ])
        );
        assert_eq!(
            parse_args(["--", "--odd.pdf"]).unwrap(),
            CliRequest::Run(vec![PathBuf::from("--odd.pdf")])
        );
    }

    #[test]
    fn unknown_flag_is_an_error() {
        assert_eq!(
            parse_args(["--nope"]).unwrap_err(),
            CliError::Unknown("--nope".into())
        );
    }
}
