use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use neural_memory_personal::embeddings::{
    DeterministicTestEmbedder, EmbeddingProfile, LlamaCppEmbedder, PersonalEmbedder,
};
use neural_memory_personal::PersonalStore;

fn arguments(values: &[String]) -> Result<HashMap<String, String>, String> {
    let mut output = HashMap::new();
    let mut index = 0;
    while index < values.len() {
        let key = values[index]
            .strip_prefix("--")
            .ok_or("unexpected positional argument")?;
        let value = values.get(index + 1).ok_or("flag value required")?;
        if output.insert(key.into(), value.clone()).is_some() {
            return Err("duplicate flag".into());
        }
        index += 2;
    }
    Ok(output)
}

fn take(map: &mut HashMap<String, String>, name: &str) -> Result<String, String> {
    map.remove(name)
        .ok_or_else(|| format!("--{name} is required"))
}

fn run() -> Result<serde_json::Value, String> {
    let values: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = values.split_first().ok_or("command required")?;
    let mut args = arguments(rest)?;
    let database = PathBuf::from(take(&mut args, "db")?);
    if database.file_name().and_then(|value| value.to_str()) != Some("personal.db") {
        return Err("--db must name personal.db".into());
    }
    let mut store = PersonalStore::open(&database).map_err(|error| error.to_string())?;
    let result = match command.as_str() {
        "status" => serde_json::to_value(
            store
                .embedding_status(&database)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?,
        "context" => store
            .local_context(
                &take(&mut args, "query")?,
                take(&mut args, "limit")?
                    .parse()
                    .map_err(|_| "invalid limit")?,
            )
            .map_err(|error| error.to_string())?,
        "profile-set" => {
            let profile = EmbeddingProfile {
                backend: take(&mut args, "backend")?,
                model_artifact: take(&mut args, "model-artifact")?,
                dimension: take(&mut args, "dimension")?
                    .parse()
                    .map_err(|_| "invalid dimension")?,
                normalization: take(&mut args, "normalization")?,
                version: take(&mut args, "version")?,
                adapter: take(&mut args, "adapter")?,
                endpoint: args.remove("endpoint"),
            };
            if profile.adapter == "llama-cpp-http" {
                LlamaCppEmbedder::new(profile.clone())?.probe()?;
            }
            let identity = store
                .set_embedding_profile(&profile, &take(&mut args, "at")?)
                .map_err(|error| error.to_string())?;
            serde_json::json!({"profileIdentity":identity,"queued":true})
        }
        "rebuild" => {
            let limit: usize = take(&mut args, "limit")?
                .parse()
                .map_err(|_| "invalid limit")?;
            let at = take(&mut args, "at")?;
            let adapter = take(&mut args, "adapter")?;
            let completed = if adapter == "deterministic-test" {
                if take(&mut args, "allow-test-backend")? != "yes" {
                    return Err("test backend requires --allow-test-backend yes".into());
                }
                let dimension = take(&mut args, "dimension")?
                    .parse()
                    .map_err(|_| "invalid dimension")?;
                store.rebuild_embeddings(&DeterministicTestEmbedder::new(dimension), limit, &at)
            } else if adapter == "llama-cpp-http" {
                let profile = EmbeddingProfile {
                    backend: take(&mut args, "backend")?,
                    model_artifact: take(&mut args, "model-artifact")?,
                    dimension: take(&mut args, "dimension")?
                        .parse()
                        .map_err(|_| "invalid dimension")?,
                    normalization: take(&mut args, "normalization")?,
                    version: take(&mut args, "version")?,
                    adapter,
                    endpoint: Some(take(&mut args, "endpoint")?),
                };
                let embedder = LlamaCppEmbedder::new(profile)?;
                store.rebuild_embeddings(&embedder, limit, &at)
            } else {
                return Err("unknown embedding adapter".into());
            }
            .map_err(|error| error.to_string())?;
            serde_json::json!({"completed":completed})
        }
        _ => return Err("unknown local admin command".into()),
    };
    if !args.is_empty() {
        return Err("unknown flag".into());
    }
    Ok(result)
}

fn main() -> ExitCode {
    match run() {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("personal admin rejected: {error}");
            ExitCode::from(2)
        }
    }
}
