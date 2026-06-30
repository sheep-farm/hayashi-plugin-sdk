use std::collections::HashMap;

use crate::error::HayashiError;
use arrow::array::Array;

// =============================================================================
// HayashiValue
// =============================================================================

/// Espelho do enum `Value` do Hayashi. Representa qualquer valor trocado entre
/// o host (Hayashi) e o plugin.
///
/// DataFrames chegam serializados como `Dict` de arrays de coluna (chave =
/// nome da coluna, valor = `List`). Uma futura versão do SDK pode oferecer um
/// wrapper `HayashiDataFrame` sobre essa representação.
#[derive(Debug, Clone, PartialEq)]
pub enum HayashiValue {
    Float(f64),
    Int(i64),
    Bool(bool),
    Str(String),
    List(Vec<HayashiValue>),
    Dict(HashMap<String, HayashiValue>),
    Arrow(usize, usize),
    Nil,
}

impl HayashiValue {
    /// Retorna o nome do tipo como string (para mensagens de erro).
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Float(_) => "float",
            Self::Int(_) => "int",
            Self::Bool(_) => "bool",
            Self::Str(_) => "string",
            Self::List(_) => "list",
            Self::Dict(_) => "dict",
            Self::Arrow(_, _) => "arrow_array",
            Self::Nil => "nil",
        }
    }

    /// Serializa o valor para uma `String` JSON, pronta para ser retornada ao
    /// host via C ABI.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.to_serde())
            .unwrap_or_else(|_| "null".to_owned())
    }

    /// Converte para `serde_json::Value`.
    pub(crate) fn to_serde(&self) -> serde_json::Value {
        match self {
            Self::Float(f) => serde_json::json!(f),
            Self::Int(i) => serde_json::json!(i),
            Self::Bool(b) => serde_json::json!(b),
            Self::Str(s) => serde_json::json!(s),
            Self::Nil => serde_json::Value::Null,
            Self::List(lst) => {
                serde_json::Value::Array(lst.iter().map(Self::to_serde).collect())
            }
            Self::Dict(map) => {
                let obj: serde_json::Map<_, _> =
                    map.iter().map(|(k, v)| (k.clone(), v.to_serde())).collect();
                serde_json::Value::Object(obj)
            }
            Self::Arrow(arr_ptr, sch_ptr) => {
                let mut map = serde_json::Map::new();
                map.insert("__arrow_array_ptr__".to_string(), serde_json::json!(arr_ptr));
                map.insert("__arrow_schema_ptr__".to_string(), serde_json::json!(sch_ptr));
                serde_json::Value::Object(map)
            }
        }
    }

    /// Constrói um `HayashiValue` a partir de um `serde_json::Value`.
    pub(crate) fn from_serde(jval: &serde_json::Value) -> Result<Self, HayashiError> {
        Ok(match jval {
            serde_json::Value::Null => Self::Nil,
            serde_json::Value::Bool(b) => Self::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Self::Int(i)
                } else {
                    Self::Float(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            serde_json::Value::String(s) => Self::Str(s.clone()),
            serde_json::Value::Array(arr) => Self::List(
                arr.iter()
                    .map(Self::from_serde)
                    .collect::<Result<_, _>>()?,
            ),
            serde_json::Value::Object(obj) => {
                if let (Some(arr_val), Some(sch_val)) = (obj.get("__arrow_array_ptr__"), obj.get("__arrow_schema_ptr__")) {
                    if let (Some(arr_ptr), Some(sch_ptr)) = (arr_val.as_u64(), sch_val.as_u64()) {
                        return Ok(Self::Arrow(arr_ptr as usize, sch_ptr as usize));
                    }
                }
                Self::Dict(
                    obj.iter()
                        .map(|(k, v)| Self::from_serde(v).map(|vv| (k.clone(), vv)))
                        .collect::<Result<_, _>>()?,
                )
            }
        })
    }
}

// =============================================================================
// FromHayashi
// =============================================================================

/// Conversão de `HayashiValue` → tipo Rust.
///
/// Implementado para os tipos mais comuns. Implemente este trait para seus
/// próprios tipos se precisar de conversões customizadas.
pub trait FromHayashi: Sized {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError>;
}

// --- Numéricos ---------------------------------------------------------------

impl FromHayashi for f64 {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::Float(f) => Ok(f),
            HayashiValue::Int(i) => Ok(i as f64),
            HayashiValue::Bool(b) => Ok(if b { 1.0 } else { 0.0 }),
            other => Err(HayashiError::Type {
                expected: "float".into(),
                got: other.type_name().into(),
            }),
        }
    }
}

impl FromHayashi for f32 {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        f64::from_hayashi(val).map(|f| f as f32)
    }
}

impl FromHayashi for i64 {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::Int(i) => Ok(i),
            HayashiValue::Float(f) => Ok(f as i64),
            HayashiValue::Bool(b) => Ok(b as i64),
            other => Err(HayashiError::Type {
                expected: "int".into(),
                got: other.type_name().into(),
            }),
        }
    }
}

impl FromHayashi for i32 {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        i64::from_hayashi(val).map(|i| i as i32)
    }
}

impl FromHayashi for usize {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        let i = i64::from_hayashi(val)?;
        if i < 0 {
            return Err(HayashiError::Custom(format!(
                "cannot convert negative integer {i} to usize"
            )));
        }
        Ok(i as usize)
    }
}

// --- Booleano ----------------------------------------------------------------

impl FromHayashi for bool {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::Bool(b) => Ok(b),
            HayashiValue::Int(0) => Ok(false),
            HayashiValue::Int(_) => Ok(true),
            HayashiValue::Float(f) => Ok(f != 0.0),
            other => Err(HayashiError::Type {
                expected: "bool".into(),
                got: other.type_name().into(),
            }),
        }
    }
}

// --- String ------------------------------------------------------------------

impl FromHayashi for String {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::Str(s) => Ok(s),
            HayashiValue::Float(f) => Ok(f.to_string()),
            HayashiValue::Int(i) => Ok(i.to_string()),
            HayashiValue::Bool(b) => Ok(b.to_string()),
            other => Err(HayashiError::Type {
                expected: "string".into(),
                got: other.type_name().into(),
            }),
        }
    }
}

// --- Coleções ----------------------------------------------------------------

fn arrow_to_hayashi_values(array: &arrow::array::ArrayRef) -> Result<Vec<HayashiValue>, HayashiError> {
    let len = array.len();
    let mut values = Vec::with_capacity(len);
    
    match array.data_type() {
        arrow::datatypes::DataType::Float64 => {
            let arr = array.as_any().downcast_ref::<arrow::array::Float64Array>()
                .ok_or_else(|| HayashiError::Custom("failed to downcast Float64Array".into()))?;
            for i in 0..len {
                if arr.is_null(i) {
                    values.push(HayashiValue::Nil);
                } else {
                    values.push(HayashiValue::Float(arr.value(i)));
                }
            }
        }
        arrow::datatypes::DataType::Int64 => {
            let arr = array.as_any().downcast_ref::<arrow::array::Int64Array>()
                .ok_or_else(|| HayashiError::Custom("failed to downcast Int64Array".into()))?;
            for i in 0..len {
                if arr.is_null(i) {
                    values.push(HayashiValue::Nil);
                } else {
                    values.push(HayashiValue::Int(arr.value(i)));
                }
            }
        }
        arrow::datatypes::DataType::Boolean => {
            let arr = array.as_any().downcast_ref::<arrow::array::BooleanArray>()
                .ok_or_else(|| HayashiError::Custom("failed to downcast BooleanArray".into()))?;
            for i in 0..len {
                if arr.is_null(i) {
                    values.push(HayashiValue::Nil);
                } else {
                    values.push(HayashiValue::Bool(arr.value(i)));
                }
            }
        }
        arrow::datatypes::DataType::Utf8 => {
            let arr = array.as_any().downcast_ref::<arrow::array::StringArray>()
                .ok_or_else(|| HayashiError::Custom("failed to downcast StringArray".into()))?;
            for i in 0..len {
                if arr.is_null(i) {
                    values.push(HayashiValue::Nil);
                } else {
                    values.push(HayashiValue::Str(arr.value(i).to_string()));
                }
            }
        }
        other => return Err(HayashiError::Custom(format!("unsupported Arrow type for conversion: {:?}", other))),
    }
    
    Ok(values)
}

impl<T: FromHayashi> FromHayashi for Vec<T> {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::List(lst) => lst.into_iter().map(T::from_hayashi).collect(),
            HayashiValue::Arrow(array_ptr, schema_ptr) => {
                let array = <arrow::array::ArrayRef as FromHayashi>::from_hayashi(HayashiValue::Arrow(array_ptr, schema_ptr))?;
                let values = arrow_to_hayashi_values(&array)?;
                values.into_iter().map(T::from_hayashi).collect()
            }
            other => Err(HayashiError::Type {
                expected: "list or arrow_array".into(),
                got: other.type_name().into(),
            }),
        }
    }
}

impl<V: FromHayashi> FromHayashi for HashMap<String, V> {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::Dict(map) => map
                .into_iter()
                .map(|(k, v)| V::from_hayashi(v).map(|vv| (k, vv)))
                .collect(),
            other => Err(HayashiError::Type {
                expected: "dict".into(),
                got: other.type_name().into(),
            }),
        }
    }
}

// --- Opcional ----------------------------------------------------------------

impl<T: FromHayashi> FromHayashi for Option<T> {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::Nil => Ok(None),
            other => T::from_hayashi(other).map(Some),
        }
    }
}

// --- HayashiValue em si ------------------------------------------------------

impl FromHayashi for HayashiValue {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        Ok(val)
    }
}

// =============================================================================
// IntoHayashi
// =============================================================================

/// Conversão de tipo Rust → `HayashiValue`.
///
/// Implemente este trait para tipos customizados que precisam ser retornados ao
/// host.
pub trait IntoHayashi {
    fn into_hayashi(self) -> HayashiValue;
}

// --- Numéricos ---------------------------------------------------------------

impl IntoHayashi for f64 {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Float(self)
    }
}

impl IntoHayashi for f32 {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Float(self as f64)
    }
}

impl IntoHayashi for i64 {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Int(self)
    }
}

impl IntoHayashi for i32 {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Int(self as i64)
    }
}

impl IntoHayashi for usize {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Int(self as i64)
    }
}

// --- Booleano ----------------------------------------------------------------

impl IntoHayashi for bool {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Bool(self)
    }
}

// --- String ------------------------------------------------------------------

impl IntoHayashi for String {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Str(self)
    }
}

impl IntoHayashi for &str {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Str(self.to_owned())
    }
}

// --- Unit --------------------------------------------------------------------

impl IntoHayashi for () {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Nil
    }
}

// --- Coleções ----------------------------------------------------------------

impl<T: IntoHayashi> IntoHayashi for Vec<T> {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::List(self.into_iter().map(IntoHayashi::into_hayashi).collect())
    }
}

impl<V: IntoHayashi> IntoHayashi for HashMap<String, V> {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Dict(
            self.into_iter()
                .map(|(k, v)| (k, v.into_hayashi()))
                .collect(),
        )
    }
}

// --- Opcional ----------------------------------------------------------------

impl<T: IntoHayashi> IntoHayashi for Option<T> {
    fn into_hayashi(self) -> HayashiValue {
        match self {
            Some(v) => v.into_hayashi(),
            None => HayashiValue::Nil,
        }
    }
}

// --- HayashiValue em si ------------------------------------------------------

impl IntoHayashi for HayashiValue {
    fn into_hayashi(self) -> HayashiValue {
        self
    }
}

// --- Arrow FFI ---------------------------------------------------------------

impl FromHayashi for arrow::array::ArrayRef {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::Arrow(array_ptr, schema_ptr) => {
                let array_ptr = array_ptr as *mut arrow::ffi::FFI_ArrowArray;
                let schema_ptr = schema_ptr as *mut arrow::ffi::FFI_ArrowSchema;
                unsafe {
                    let array_data = arrow::ffi::from_ffi(std::ptr::read(array_ptr), &*schema_ptr)
                        .map_err(|e| HayashiError::Custom(format!("failed to import Arrow array: {e}")))?;
                    
                    Ok(arrow::array::make_array(array_data))
                }
            }
            other => Err(HayashiError::Type {
                expected: "arrow_array".into(),
                got: other.type_name().into(),
            }),
        }
    }
}

impl IntoHayashi for arrow::array::ArrayRef {
    fn into_hayashi(self) -> HayashiValue {
        match arrow::ffi::to_ffi(&self.into_data()) {
            Ok((ffi_array, ffi_schema)) => {
                let array_ptr = Box::into_raw(Box::new(ffi_array));
                let schema_ptr = Box::into_raw(Box::new(ffi_schema));
                HayashiValue::Arrow(array_ptr as usize, schema_ptr as usize)
            }
            Err(_) => HayashiValue::Nil,
        }
    }
}

