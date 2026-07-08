# hayashi-plugin-sdk

SDK para criar plugins nativos (Rust `.so`/`.dll`) e WebAssembly (`.wasm`) para a
linguagem [Hayashi](https://github.com/sheep-farm/hayashi).

## Instalação

```toml
# Cargo.toml do seu plugin
[lib]
crate-type = ["cdylib"]          # para plugin nativo
# crate-type = ["cdylib", "rlib"] # para nativo + testes unitários

[dependencies]
hayashi-plugin-sdk = "0.1"
```

## Exemplo mínimo

```rust
use hayashi_plugin_sdk::{hayashi_fn, hayashi_plugin};

/// Índice de Sharpe anualizado.
#[hayashi_fn]
pub fn sharpe_ratio(returns: Vec<f64>, rf: f64) -> f64 {
    let n    = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let std  = (returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n).sqrt();
    (mean - rf) / std
}

/// Máximo drawdown.
#[hayashi_fn]
pub fn max_drawdown(returns: Vec<f64>) -> f64 {
    let mut peak = f64::NEG_INFINITY;
    let mut max_dd = 0.0_f64;
    let mut equity = 1.0_f64;
    for r in &returns {
        equity *= 1.0 + r;
        if equity > peak { peak = equity; }
        let dd = (peak - equity) / peak;
        if dd > max_dd { max_dd = dd; }
    }
    max_dd
}

// Gera o símbolo `free_string` que o Hayashi usa para liberar memória.
// Deve aparecer exatamente uma vez por plugin.
hayashi_plugin!();
```

No script `.hay`:

```
import_native("sheep-farm/hayashi-finance")

load "returns.csv" as df
let sr = sharpe_ratio(df["ret"], 0.0)
let dd = max_drawdown(df["ret"])
print(f"Sharpe: {sr:.3f}  MaxDD: {dd:.2%}")
```

## Referência

### `#[hayashi_fn]`

Decora qualquer função `pub fn` e gera automaticamente:

- A função original renomeada para `__hayashi_impl_<nome>` (privada, para testes)
- Um `#[no_mangle] pub extern "C" fn <nome>(...)` que:
  - Recebe os argumentos como JSON array em `*const c_char`
  - Deserializa via `FromHayashi`
  - Chama a função original
  - Serializa o retorno via `IntoHayashi` em `*mut c_char`
  - Envolve tudo em `catch_unwind` (panics não cruzam a ABI C)

### Tipos suportados

| Tipo Rust              | Tipo Hayashi  |
|------------------------|---------------|
| `f64`, `f32`           | float         |
| `i64`, `i32`, `usize`  | int           |
| `bool`                 | bool          |
| `String`               | string        |
| `Vec<T>`               | list          |
| `HashMap<String, V>`   | dict          |
| `Option<T>`            | nil / valor   |
| `HayashiValue`         | any           |

### Retornos

| Assinatura             | Comportamento                                         |
|------------------------|-------------------------------------------------------|
| `-> T`                 | Serializado diretamente                               |
| `-> Result<T, E>`      | `Ok(v)` serializado; `Err(e)` → `{"__error__":"..."}` |
| `-> ()`                | Retorna `nil`                                         |

### `hayashi_plugin!()`

Macro declarativa que gera o símbolo `free_string`. Deve ser invocada exatamente
uma vez no crate raiz do plugin.

## Retornando erros

```rust
#[hayashi_fn]
pub fn safe_log(x: f64) -> Result<f64, String> {
    if x <= 0.0 {
        return Err(format!("log of non-positive value: {x}"));
    }
    Ok(x.ln())
}
```

No Hayashi, erros são capturáveis com `try { } catch e { }`:

```
try {
    let v = safe_log(-1.0)
} catch e {
    display f"Erro: {e}"
}
```

## Parâmetros opcionais

```rust
#[hayashi_fn]
pub fn weighted_mean(values: Vec<f64>, weights: Option<Vec<f64>>) -> f64 {
    match weights {
        Some(w) => {
            let total: f64 = w.iter().sum();
            values.iter().zip(w.iter()).map(|(v, ww)| v * ww).sum::<f64>() / total
        }
        None => values.iter().sum::<f64>() / values.len() as f64,
    }
}
```

```
let m1 = weighted_mean(df["ret"])             // weights = nil → média simples
let m2 = weighted_mean(df["ret"], df["mktcap"]) // ponderado por capitalização
```

## DataFrames

DataFrames chegam ao plugin como `HayashiValue::Dict` onde cada chave é o nome
de uma coluna e o valor é uma `HayashiValue::List`. Use `HayashiValue` diretamente
quando precisar receber um DataFrame:

```rust
use hayashi_plugin_sdk::{hayashi_fn, hayashi_plugin, HayashiValue, HayashiError};
use std::collections::HashMap;

#[hayashi_fn]
pub fn column_stats(df: HashMap<String, Vec<f64>>) -> HashMap<String, f64> {
    df.into_iter()
        .map(|(col, vals)| {
            let mean = vals.iter().sum::<f64>() / vals.len() as f64;
            (col, mean)
        })
        .collect()
}

hayashi_plugin!();
```

## Testes unitários

Como o `#[hayashi_fn]` preserva a função original (renomeada para
`__hayashi_impl_<nome>`), você pode testá-la diretamente sem simular JSON:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sharpe() {
        let returns = vec![0.01, 0.02, -0.01, 0.03, 0.00];
        let sr = __hayashi_impl_sharpe_ratio(returns, 0.0);
        assert!(sr > 0.0);
    }
}
```

## Publicar no GitHub

Plugins devem ser repositórios GitHub **públicos**. O `hay install` baixa os
binários diretamente dos GitHub Releases gerados pela CI do repositório — não
aceita binários compilados manualmente.

Template de CI recomendado:

```yaml
# .github/workflows/release.yml
name: Release

on:
  push:
    tags: ["v*"]

jobs:
  build:
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            ext: so
          - os: macos-latest
            target: aarch64-apple-darwin
            ext: dylib
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            ext: dll

    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Build
        run: |
          cargo build --release --target ${{ matrix.target }}

      - name: Rename artifact
        run: |
          mv target/${{ matrix.target }}/release/libmyplugin.${{ matrix.ext }} \
             myplugin-${{ matrix.target }}.${{ matrix.ext }}

      - name: Attest build provenance
        uses: actions/attest-build-provenance@v1
        with:
          subject-path: myplugin-${{ matrix.target }}.${{ matrix.ext }}

      - name: Upload to Release
        uses: softprops/action-gh-release@v2
        with:
          files: myplugin-${{ matrix.target }}.${{ matrix.ext }}
```

## Modelos econométricos

Quando um modelo estimado (e.g. resultado de `ols()`, `logit()`, `gmm()`) é
passado para um plugin nativo, o Hayashi o serializa como `HayashiValue::Dict`
com os seguintes campos:

| Campo            | Tipo     | Descrição                                  |
|------------------|----------|--------------------------------------------|
| `__model_type__` | `Str`    | Tipo do modelo (`"ols"`, `"logit"`, etc.)  |
| `variable`       | `List`   | Nomes das variáveis                        |
| `coef`           | `List`   | Coeficientes estimados                     |
| `std_err`        | `List`   | Erros padrão                               |
| `t` ou `z`       | `List`   | Estatísticas t (OLS/IV) ou z (MLE)         |
| `p_value`        | `List`   | P-valores                                  |
| `conf_low`       | `List`   | Limite inferior do IC (OLS apenas)         |
| `conf_high`      | `List`   | Limite superior do IC (OLS apenas)         |
| `r2` / `pseudo_r2` | `Float` | R² ou pseudo-R²                          |
| `n`              | `Float`  | Número de observações                      |
| `aic`, `bic`     | `Float`  | Critérios de informação                    |
| `log_lik`        | `Float`  | Log-likelihood                             |
| `sigma`          | `Float`  | Erro padrão da regressão                   |

Os campos de ajuste variam por tipo de modelo. Campos ausentes simplesmente
não aparecem no dict.

```rust
use hayashi_plugin_sdk::{hayashi_fn, hayashi_plugin, HayashiValue};
use std::collections::HashMap;

#[hayashi_fn]
pub fn n_obs(model: HayashiValue) -> String {
    match &model {
        HayashiValue::Dict(d) => {
            if let Some(HayashiValue::Float(n)) = d.get("n") {
                format!("Observations: {}", *n as i64)
            } else {
                "N/A".to_string()
            }
        }
        _ => "not a model".to_string(),
    }
}

hayashi_plugin!();
```

Modelos suportados: OLS, IV/2SLS, Logit/Probit, Panel FE/RE, GMM, Poisson,
NegBin, GLM, Quantile, Tobit, Heckman, Ordered, Arellano-Bond, Ridge/Lasso/
ElasticNet, RLM, Beta, GEE, ARIMA, GARCH.

## Limitações (v0.1)

- Apenas parâmetros com nomes simples (sem destructuring)
- DataFrames passados como `Dict` de listas ou Arrow FFI — sem tipo `HayashiDataFrame` nativo
- Sem suporte a parâmetros nomeados (`opt=value`) — use `Option<T>` posicional
- Plugins WASM seguem protocolo diferente (ver `wasmi` em `plugin.rs` do Hayashi)

## Licença

MIT
