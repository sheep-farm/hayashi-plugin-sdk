use crate::error::HayashiError;
use crate::value::{FromHayashi, HayashiValue};

/// Faz o parse do JSON array recebido do host e retorna um `Vec<HayashiValue>`.
///
/// O host sempre envia os argumentos como um array JSON:
/// `[arg0, arg1, arg2, ...]`
pub fn parse_args(json: &str) -> Result<Vec<HayashiValue>, HayashiError> {
    let jval: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| HayashiError::Parse(e.to_string()))?;

    match jval {
        serde_json::Value::Array(arr) => arr
            .iter()
            .map(HayashiValue::from_serde)
            .collect::<Result<_, _>>(),
        _ => Err(HayashiError::Parse(
            "expected a JSON array of arguments".into(),
        )),
    }
}

/// Extrai o próximo argumento do iterador e converte para `T`.
///
/// Se o iterador estiver vazio, usa `HayashiValue::Nil` como fallback
/// (parâmetros opcionais podem usar `Option<T>`).
///
/// O `index` é usado apenas para mensagens de erro mais informativas.
pub fn extract_arg<T: FromHayashi>(
    iter: &mut impl Iterator<Item = HayashiValue>,
    index: usize,
) -> Result<T, HayashiError> {
    let val = iter.next().unwrap_or(HayashiValue::Nil);
    T::from_hayashi(val).map_err(|e| HayashiError::Arg {
        index,
        message: e.to_string(),
    })
}
