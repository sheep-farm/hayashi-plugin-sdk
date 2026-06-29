use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, FnArg, ItemFn, Pat, ReturnType, Type};

// =============================================================================
// #[hayashi_fn]
// =============================================================================

/// Transforma uma função Rust comum em um export C ABI compatível com o
/// sistema de plugins nativos do Hayashi.
///
/// A macro:
/// 1. Renomeia a função original para `__hayashi_impl_<nome>` (fica privada)
/// 2. Gera um `#[no_mangle] pub extern "C" fn <nome>(...)` que:
///    - Recebe os argumentos como um JSON array em `*const c_char`
///    - Deserializa cada argumento via `FromHayashi`
///    - Chama a função original
///    - Serializa o retorno via `IntoHayashi` e o devolve como `*mut c_char`
///    - Envolve tudo em `catch_unwind` para não deixar panics cruzarem a
///      fronteira C ABI
///
/// ## Tipos suportados como parâmetros
///
/// | Tipo Rust          | Tipo Hayashi |
/// |--------------------|--------------|
/// | `f64`, `f32`       | float        |
/// | `i64`, `i32`, `usize` | int       |
/// | `bool`             | bool         |
/// | `String`           | string       |
/// | `Vec<T>`           | list         |
/// | `HashMap<String,V>`| dict         |
/// | `Option<T>`        | nil / valor  |
/// | `HayashiValue`     | any          |
///
/// ## Tipos de retorno suportados
///
/// | Retorno            | Comportamento                             |
/// |--------------------|-------------------------------------------|
/// | `T: IntoHayashi`   | serializado diretamente                   |
/// | `Result<T, E>`     | `Ok(v)` serializado; `Err(e)` → `{"__error__":"..."}` |
/// | `()`               | retorna `nil`                             |
///
/// ## Exemplo
///
/// ```rust,ignore
/// #[hayashi_fn]
/// pub fn sharpe_ratio(returns: Vec<f64>, rf: f64) -> f64 {
///     let mean = returns.iter().sum::<f64>() / returns.len() as f64;
///     let std  = (returns.iter()
///                    .map(|r| (r - mean).powi(2))
///                    .sum::<f64>() / returns.len() as f64).sqrt();
///     (mean - rf) / std
/// }
/// ```
///
/// Com `Result`:
/// ```rust,ignore
/// #[hayashi_fn]
/// pub fn safe_div(a: f64, b: f64) -> Result<f64, String> {
///     if b == 0.0 {
///         return Err("division by zero".into());
///     }
///     Ok(a / b)
/// }
/// ```
#[proc_macro_attribute]
pub fn hayashi_fn(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    expand_hayashi_fn(input)
        .unwrap_or_else(|e| e.to_compile_error().into())
}

// =============================================================================
// Implementação interna
// =============================================================================

fn expand_hayashi_fn(input: ItemFn) -> syn::Result<TokenStream> {
    let vis = &input.vis;
    let fn_name = &input.sig.ident;

    // Nome da função interna (privada): __hayashi_impl_<fn_name>
    let impl_name = syn::Ident::new(
        &format!("__hayashi_impl_{}", fn_name),
        Span::call_site(),
    );

    // Coleta apenas parâmetros tipados (ignora `self`)
    let typed_params: Vec<&syn::PatType> = input
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(pt) => Some(pt),
            FnArg::Receiver(_) => None,
        })
        .collect();

    // Nomes dos parâmetros (aceita apenas `ident: Type`, não patterns complexos)
    let param_names: Vec<syn::Ident> = typed_params
        .iter()
        .enumerate()
        .map(|(_i, pt)| match pt.pat.as_ref() {
            Pat::Ident(pi) => Ok(pi.ident.clone()),
            _ => Err(syn::Error::new_spanned(
                &pt.pat,
                "#[hayashi_fn]: only simple identifier patterns are supported as parameter names",
            )),
        })
        .collect::<syn::Result<_>>()?;

    // Tipos dos parâmetros (tokens originais, passados ao `extract_arg`)
    let param_types: Vec<&Type> = typed_params.iter().map(|pt| pt.ty.as_ref()).collect();

    // Índices para `extract_arg` (mensagens de erro mais informativas)
    let param_indices: Vec<usize> = (0..typed_params.len()).collect();

    // Analisa o tipo de retorno
    let return_kind = classify_return(&input.sig.output);

    // Gera o corpo da closure interna que chama a função original
    let call_expr = generate_call(&impl_name, &param_names, return_kind);

    // Cria a função interna (renomeada, privada)
    let mut impl_fn = input.clone();
    impl_fn.sig.ident = impl_name;
    impl_fn.vis = syn::Visibility::Inherited;

    let expanded = quote! {
        // Função original, renomeada e privada.
        #impl_fn

        // Export C ABI gerado automaticamente pelo #[hayashi_fn].
        #[no_mangle]
        #vis extern "C" fn #fn_name(
            __args_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            // Converte o ponteiro para String ANTES do catch_unwind:
            // raw pointers não são UnwindSafe.
            let __args_str: ::std::string::String = unsafe {
                ::std::ffi::CStr::from_ptr(__args_json)
                    .to_str()
                    .unwrap_or("[]")
                    .to_owned()
            };

            let __output: ::std::string::String = match ::std::panic::catch_unwind(
                move || -> ::std::result::Result<
                    ::std::string::String,
                    ::hayashi_plugin_sdk::HayashiError,
                > {
                    let mut __args = ::hayashi_plugin_sdk::parse_args(&__args_str)?;
                    let mut __iter = __args.drain(..);

                    // Extrai e converte cada argumento posicional.
                    #(
                        let #param_names: #param_types =
                            ::hayashi_plugin_sdk::extract_arg(&mut __iter, #param_indices)?;
                    )*

                    #call_expr
                },
            ) {
                ::std::result::Result::Ok(::std::result::Result::Ok(s)) => s,
                ::std::result::Result::Ok(::std::result::Result::Err(e)) => {
                    ::std::format!(r#"{{"__error__":"{}"}}"#, e)
                }
                ::std::result::Result::Err(_) => {
                    r#"{"__error__":"plugin panicked"}"#.to_owned()
                }
            };

            ::std::ffi::CString::new(__output)
                .unwrap_or_else(|_| {
                    ::std::ffi::CString::new(
                        r#"{"__error__":"plugin returned invalid UTF-8"}"#,
                    )
                    .unwrap()
                })
                .into_raw()
        }
    };

    Ok(expanded.into())
}

// =============================================================================
// Classificação do tipo de retorno
// =============================================================================

#[derive(Clone, Copy, PartialEq)]
enum ReturnKind {
    /// `fn foo() { }` ou `-> ()`
    Unit,
    /// `-> Result<T, E>`
    Result,
    /// `-> T` onde T: IntoHayashi
    Value,
}

fn classify_return(ret: &ReturnType) -> ReturnKind {
    match ret {
        ReturnType::Default => ReturnKind::Unit,
        ReturnType::Type(_, ty) => {
            if is_result_type(ty) {
                ReturnKind::Result
            } else if is_unit_type(ty) {
                ReturnKind::Unit
            } else {
                ReturnKind::Value
            }
        }
    }
}

/// Verifica se o último segmento do caminho é `Result`.
/// Cobre: `Result<T, E>`, `std::result::Result<T, E>`, etc.
fn is_result_type(ty: &Type) -> bool {
    if let Type::Path(tp) = ty {
        if let Some(last) = tp.path.segments.last() {
            return last.ident == "Result";
        }
    }
    false
}

/// Verifica se o tipo é `()` (unit tuple).
fn is_unit_type(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(t) if t.elems.is_empty())
}

// =============================================================================
// Geração da chamada à função original
// =============================================================================

fn generate_call(
    impl_name: &syn::Ident,
    param_names: &[syn::Ident],
    kind: ReturnKind,
) -> proc_macro2::TokenStream {
    match kind {
        ReturnKind::Unit => quote! {
            #impl_name(#(#param_names),*);
            ::std::result::Result::Ok(
                ::hayashi_plugin_sdk::HayashiValue::Nil.to_json(),
            )
        },

        ReturnKind::Value => quote! {
            let __ret = #impl_name(#(#param_names),*);
            ::std::result::Result::Ok(
                ::hayashi_plugin_sdk::IntoHayashi::into_hayashi(__ret).to_json(),
            )
        },

        ReturnKind::Result => quote! {
            let __ret = #impl_name(#(#param_names),*);
            let __ret = __ret.map_err(|e| {
                ::hayashi_plugin_sdk::HayashiError::Function(e.to_string())
            })?;
            ::std::result::Result::Ok(
                ::hayashi_plugin_sdk::IntoHayashi::into_hayashi(__ret).to_json(),
            )
        },
    }
}
