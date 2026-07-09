//! # hayashi-plugin-sdk
//!
//! SDK para criar plugins nativos (Rust `.so`/`.dll`) e WebAssembly (`.wasm`)
//! para a linguagem [Hayashi](https://github.com/sheep-farm/hayashi).
//!
//! ## Uso rápido
//!
//! ```toml
//! # Cargo.toml do seu plugin
//! [lib]
//! crate-type = ["cdylib"]
//!
//! [dependencies]
//! hayashi-plugin-sdk = "0.1"
//! ```
//!
//! ```rust,ignore
//! use hayashi_plugin_sdk::{hayashi_fn, hayashi_plugin};
//!
//! // Gera o boilerplate C ABI automaticamente
//! #[hayashi_fn]
//! pub fn sharpe_ratio(returns: Vec<f64>, rf: f64) -> f64 {
//!     let mean = returns.iter().sum::<f64>() / returns.len() as f64;
//!     let std  = (returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>()
//!                / returns.len() as f64).sqrt();
//!     (mean - rf) / std
//! }
//!
//! // Gera o `free_string` que o Hayashi usa para liberar memória
//! hayashi_plugin!();
//! ```
//!
//! No script `.hay`:
//! ```text
//! import_native("sheep-farm/hayashi-finance")
//! let sr = sharpe_ratio(ret, 0.02)
//! ```

pub mod error;
pub mod ffi;
pub mod value;

// Re-exports públicos — o autor do plugin só precisa importar o crate raiz.
pub use error::HayashiError;
pub use ffi::{extract_arg, parse_args};
pub use value::{FromHayashi, Geometry, HayashiValue, IntoHayashi, Plot};

#[cfg(feature = "seed")]
pub use value::Seed;

// Re-export do crate arrow para que plugins possam usar FFI sem adicioná-lo ao Cargo.toml
pub use arrow;

// Re-export do proc macro do sub-crate
pub use hayashi_plugin_sdk_macros::hayashi_fn;

/// Gera os símbolos FFI de liberação de memória necessários pelo host Hayashi.
///
/// **Deve ser invocado exatamente uma vez** no crate raiz do plugin.
///
/// # Exemplo
///
/// ```rust,ignore
/// hayashi_plugin_sdk::hayashi_plugin!();
/// ```
#[macro_export]
macro_rules! hayashi_plugin {
    () => {
        /// Libera uma string alocada pelo plugin e retornada ao host via C ABI.
        #[no_mangle]
        pub extern "C" fn free_string(ptr: *mut ::std::os::raw::c_char) {
            if !ptr.is_null() {
                unsafe {
                    drop(::std::ffi::CString::from_raw(ptr));
                }
            }
        }

        /// Libera as estruturas FFI do Arrow alocadas no heap do guest e retornadas ao host.
        #[no_mangle]
        pub extern "C" fn free_arrow_pointers(
            array_ptr: *mut $crate::arrow::ffi::FFI_ArrowArray,
            schema_ptr: *mut $crate::arrow::ffi::FFI_ArrowSchema,
        ) {
            if !array_ptr.is_null() {
                unsafe {
                    let mut arr_box = ::std::boxed::Box::from_raw(array_ptr);
                    arr_box.release = None;
                    drop(arr_box);
                }
            }
            if !schema_ptr.is_null() {
                unsafe {
                    let mut sch_box = ::std::boxed::Box::from_raw(schema_ptr);
                    sch_box.release = None;
                    drop(sch_box);
                }
            }
        }
    };
}
