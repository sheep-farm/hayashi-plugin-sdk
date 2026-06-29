/// Erros que podem ocorrer durante a execução de um plugin Hayashi.
#[derive(Debug)]
pub enum HayashiError {
    /// Falha ao fazer parse do JSON recebido do host.
    Parse(String),

    /// Tipo incompatível na conversão de `HayashiValue`.
    Type {
        expected: String,
        got: String,
    },

    /// Falha na conversão de um argumento posicional específico.
    Arg {
        index: usize,
        message: String,
    },

    /// A função do plugin retornou `Err(...)`.
    Function(String),

    /// Erro customizado pelo autor do plugin.
    Custom(String),
}

impl std::fmt::Display for HayashiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::Type { expected, got } => {
                write!(f, "type error: expected {expected}, got {got}")
            }
            Self::Arg { index, message } => write!(f, "argument {index}: {message}"),
            Self::Function(msg) => write!(f, "function error: {msg}"),
            Self::Custom(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for HayashiError {}
