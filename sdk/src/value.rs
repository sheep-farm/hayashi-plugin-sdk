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
    /// Geometria vetorial em Well-Known Text (WKT).
    ///
    /// É o tipo canônico para trocar dados geoespaciais entre plugins e o host.
    /// O conteúdo é sempre uma string WKT válida, ex:
    /// `"POLYGON ((0 0, 1 0, 1 1, 0 0))"`.
    ///
    /// Plugins geoespaciais devem aceitar e retornar `Geometry` em vez de
    /// passar WKT como `Str`, garantindo que o host possa distinguir geometrias
    /// de strings comuns e compor pipelines entre plugins sem conversão manual.
    Geometry(String),
    /// Output visual composável como spec Vega-Lite (JSON).
    ///
    /// Plugins de visualização devem retornar `Plot` em vez de PNG em base64
    /// ou SVG como `Str`. O host decide como renderizar (terminal, browser,
    /// arquivo) sem precisar que o plugin conheça o destino.
    ///
    /// `format` identifica o schema da spec: `"vega-lite"`, `"plotters-svg"`,
    /// `"plotters-png-b64"`. O host pode ignorar formatos que não suporta.
    ///
    /// Layers adicionais podem ser adicionados ao mesmo `Plot` pelo host antes
    /// de renderizar, viabilizando composição `plot + geom_line + geom_point`
    /// sem round-trips de serialização.
    Plot {
        /// Especificação do plot (tipicamente JSON Vega-Lite ou SVG/PNG em b64).
        spec: String,
        /// Identificador do formato: `"vega-lite"`, `"plotters-svg"`, `"plotters-png-b64"`.
        format: String,
    },
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
            Self::Geometry(_) => "geometry",
            Self::Plot { .. } => "plot",
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
            // Geometry: marcador __geometry_wkt__ para o host distinguir de Str
            Self::Geometry(wkt) => {
                let mut map = serde_json::Map::new();
                map.insert("__geometry_wkt__".to_string(), serde_json::json!(wkt));
                serde_json::Value::Object(map)
            }
            // Plot: marcador __plot_spec__ + __plot_format__
            Self::Plot { spec, format } => {
                let mut map = serde_json::Map::new();
                map.insert("__plot_spec__".to_string(), serde_json::json!(spec));
                map.insert("__plot_format__".to_string(), serde_json::json!(format));
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
                // Arrow FFI pointers
                if let (Some(arr_val), Some(sch_val)) = (obj.get("__arrow_array_ptr__"), obj.get("__arrow_schema_ptr__")) {
                    if let (Some(arr_ptr), Some(sch_ptr)) = (arr_val.as_u64(), sch_val.as_u64()) {
                        return Ok(Self::Arrow(arr_ptr as usize, sch_ptr as usize));
                    }
                }
                // Geometry (WKT)
                if let Some(wkt_val) = obj.get("__geometry_wkt__") {
                    if let Some(wkt) = wkt_val.as_str() {
                        return Ok(Self::Geometry(wkt.to_owned()));
                    }
                }
                // Plot
                if let (Some(spec_val), Some(fmt_val)) = (obj.get("__plot_spec__"), obj.get("__plot_format__")) {
                    if let (Some(spec), Some(format)) = (spec_val.as_str(), fmt_val.as_str()) {
                        return Ok(Self::Plot {
                            spec: spec.to_owned(),
                            format: format.to_owned(),
                        });
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

// --- Geometry ----------------------------------------------------------------

/// Wrapper de nova tipagem para geometria WKT.
///
/// Use-o como parâmetro ou retorno em funções marcadas com `#[hayashi_fn]`
/// para sinalizar explicitamente ao host que o valor é geoespacial, não uma
/// string genérica.
///
/// # Exemplo
///
/// ```rust,ignore
/// #[hayashi_fn]
/// pub fn bbox(geom: Geometry) -> Geometry {
///     // ...
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry(pub String);

impl Geometry {
    /// Cria uma geometria a partir de um WKT string.
    pub fn from_wkt(wkt: impl Into<String>) -> Self {
        Self(wkt.into())
    }

    /// Retorna o WKT da geometria.
    pub fn wkt(&self) -> &str {
        &self.0
    }
}

impl FromHayashi for Geometry {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::Geometry(wkt) => Ok(Geometry(wkt)),
            // Aceita Str como fallback para compatibilidade com plugins pré-Geometry
            HayashiValue::Str(s) => Ok(Geometry(s)),
            other => Err(HayashiError::Type {
                expected: "geometry".into(),
                got: other.type_name().into(),
            }),
        }
    }
}

impl IntoHayashi for Geometry {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Geometry(self.0)
    }
}

// --- Plot --------------------------------------------------------------------

/// Wrapper para output visual composável.
///
/// Use como tipo de retorno em funções de visualização marcadas com
/// `#[hayashi_fn]`. O host decide como renderizar sem que o plugin precise
/// conhecer o destino (terminal, arquivo, browser).
///
/// # Exemplo
///
/// ```rust,ignore
/// #[hayashi_fn]
/// pub fn scatter(df: ArrayRef, x: String, y: String) -> Plot {
///     let spec = build_vega_lite_spec(&df, &x, &y);
///     Plot::vega_lite(spec)
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Plot {
    pub spec: String,
    pub format: String,
}

impl Plot {
    /// Cria um `Plot` com spec Vega-Lite.
    pub fn vega_lite(spec: impl Into<String>) -> Self {
        Self { spec: spec.into(), format: "vega-lite".into() }
    }

    /// Cria um `Plot` com SVG gerado pelo Plotters.
    pub fn plotters_svg(svg: impl Into<String>) -> Self {
        Self { spec: svg.into(), format: "plotters-svg".into() }
    }

    /// Cria um `Plot` com PNG em base64 gerado pelo Plotters.
    pub fn plotters_png_b64(b64: impl Into<String>) -> Self {
        Self { spec: b64.into(), format: "plotters-png-b64".into() }
    }
}

impl FromHayashi for Plot {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::Plot { spec, format } => Ok(Plot { spec, format }),
            other => Err(HayashiError::Type {
                expected: "plot".into(),
                got: other.type_name().into(),
            }),
        }
    }
}

impl IntoHayashi for Plot {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Plot { spec: self.spec, format: self.format }
    }
}

// --- Seed --------------------------------------------------------------------

#[cfg(feature = "seed")]
/// Semente RNG injetada pelo host quando o usuário chama `set_seed(N)`.
///
/// O host injeta `__seed__` como último argumento oculto nas chamadas a
/// plugins que declaram `seed: Option<Seed>` como parâmetro final.
///
/// Requer a feature `seed` do SDK (ativa `rand 0.10`):
/// ```toml
/// hayashi-plugin-sdk = { version = "0.1", features = ["seed"] }
/// ```
///
/// # Exemplo
///
/// ```rust,ignore
/// #[hayashi_fn]
/// pub fn monte_carlo(n: i64, seed: Option<Seed>) -> Vec<f64> {
///     let mut rng = seed
///         .map(|s| s.into_rng())
///         .unwrap_or_else(|| rand::rngs::StdRng::from_entropy());
///     // ...
/// }
/// ```
#[cfg(feature = "seed")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seed(pub u64);

#[cfg(feature = "seed")]
impl Seed {
    /// Cria um `StdRng` derivado desta semente.
    pub fn into_rng(self) -> rand::rngs::StdRng {
        use rand::SeedableRng;
        rand::rngs::StdRng::seed_from_u64(self.0)
    }
}

#[cfg(feature = "seed")]
impl FromHayashi for Seed {
    fn from_hayashi(val: HayashiValue) -> Result<Self, HayashiError> {
        match val {
            HayashiValue::Int(i) => Ok(Seed(i as u64)),
            HayashiValue::Float(f) => Ok(Seed(f as u64)),
            other => Err(HayashiError::Type {
                expected: "seed (int)".into(),
                got: other.type_name().into(),
            }),
        }
    }
}

#[cfg(feature = "seed")]
impl IntoHayashi for Seed {
    fn into_hayashi(self) -> HayashiValue {
        HayashiValue::Int(self.0 as i64)
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

