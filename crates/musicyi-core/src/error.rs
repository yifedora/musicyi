use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("配置错误: {0}")]
    Config(String),
    #[error("路径错误: {0}")]
    Path(String),
    #[error("网络错误: {0}")]
    Network(String),
    #[error("接口错误: {0}")]
    Api(String),
    #[error("音频错误: {0}")]
    Audio(String),
    #[error("存储错误: {0}")]
    Storage(String),
    #[error("{0}")]
    Other(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Other(e.to_string())
    }
}
