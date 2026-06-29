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
pub use value::{FromHayashi, HayashiValue, IntoHayashi};

// Re-export do proc macro do sub-crate
pub use hayashi_plugin_sdk_macros::hayashi_fn;

/// Gera o símbolo `free_string` que o Hayashi chama para liberar a memória das
/// strings retornadas pelas funções do plugin.
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
        ///
        /// Chamado automaticamente pelo Hayashi após consumir o valor retornado
        /// por uma função de plugin.
        #[no_mangle]
        pub extern "C" fn free_string(ptr: *mut ::std::os::raw::c_char) {
            if !ptr.is_null() {
                // SAFETY: `ptr` foi alocado por `CString::into_raw()` neste
                // mesmo processo. O Hayashi garante que `free_string` é chamado
                // exatamente uma vez por ponteiro retornado.
                unsafe {
                    drop(::std::ffi::CString::from_raw(ptr));
                }
            }
        }
    };
}
