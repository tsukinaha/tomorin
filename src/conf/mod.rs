use std::{
    fs::{create_dir_all, write},
    path::Path,
    process,
};

use miette::{IntoDiagnostic, miette};

#[derive(knuffel::Decode, Debug, PartialEq, Default)]
pub struct Conf {
    #[knuffel(child, unwrap(argument), default)]
    pub api_id: i32,
    #[knuffel(child, unwrap(argument), default)]
    pub api_hash: String,
    #[knuffel(child, unwrap(argument), default)]
    pub phone: String,
}

impl Conf {
    fn load(path: &Path) -> miette::Result<Self> {
        let contents = match std::fs::read_to_string(path).into_diagnostic() {
            Ok(contents) => contents,
            Err(err) => {
                tracing::debug!("failed to read config from {path:?}: {err}");
                return Err(err);
            }
        };

        let config: Conf = knuffel::parse(
            path.file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("config.kdl"),
            &contents,
        )
        .map_err(|e| miette!(e))?;

        tracing::debug!("loaded config from {path:?}");
        Ok(config)
    }

    pub fn load_or_create() -> miette::Result<Self> {
        const PATH: &str = "config.kdl";

        let path = Path::new(PATH);
        if !path.exists() {
            tracing::info!("config file {PATH} does not exist, creating default config");
            create_dir_all(path.parent().unwrap()).into_diagnostic()?;
            write(path, include_str!("example.kdl")).into_diagnostic()?;
            process::exit(0);
        }

        Self::load(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conf() {
        let conf = r#"
            api-id 123456
            api-hash "test_api_hash"
            phone "1234567890"
        "#;
        let conf: Conf = knuffel::parse("example.kdl", conf).unwrap();
        assert_eq!(conf.api_id, 123456);
        assert_eq!(conf.api_hash, "test_api_hash");
        assert_eq!(conf.phone, "1234567890");
    }
}
