//! Serve System One over HTTP with a local checkpoint.
//!
//! `decide` answers requests handed to it on the command line; this answers
//! them on `POST /v1/systemone`, so a client written against TypeSafe's API —
//! an agent exported from the Agent Modeler, with `RKEV_BASE_URL` pointed here —
//! talks to rkev without a code change.
//!
//! ```bash
//! cargo run --release --features candle,server --example serve -- \
//!     --base ~/models/qwen3-0.6b-base --checkpoint ~/models/kev-0.6b
//!
//! curl -s localhost:8090/v1/systemone -H 'Content-Type: application/json' -d \
//!   '{"state":"Paket kam nie an.","questions":{"decision":{"type":"choice",
//!     "instructions":"Welches Team?","criteria":{"returns":"Rueckgabe","shipping":"Lieferung"}}}}'
//! ```
//!
//! The model loads before the port opens, so a client that can connect can be
//! answered. Diagnostics go to stderr.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use candle_core::{DType, Device};
use rkev::server::Server;
use rkev::{device, option_isolation, pointer_head, Backend, LocalEngine};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> ExitCode {
    let options = match parse() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match run(options).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let loading = Instant::now();
    let dir = options.checkpoint.as_ref().unwrap_or(&options.base);
    let head_path = options.head.clone().unwrap_or_else(|| {
        // kev ships head.pt; an exported head may sit beside it as safetensors.
        [dir.join("head.pt"), dir.join("head.safetensors")]
            .into_iter()
            .find(|path| path.exists())
            .unwrap_or_else(|| dir.join("head.pt"))
    });
    // What a request's `model` may name besides kev-latest: --model, or the
    // checkpoint's directory name (kev-0.6b), which is what a person would type.
    let name = options.model.clone().unwrap_or_else(|| {
        dir.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| rkev::DEFAULT_MODEL.to_string())
    });

    let checkpoint = options.checkpoint.as_deref();
    let backend = match options.dtype {
        Some(dtype) => Backend::open_as(&options.base, checkpoint, options.device.clone(), dtype)?,
        None => Backend::open_on(&options.base, checkpoint, options.device.clone())?,
    };
    // A checkpoint records the layout it was trained on, and serving the other
    // one is a silently different prompt rather than an error.
    let isolation = match options.isolation {
        Some(chosen) => Some(chosen),
        None => option_isolation(&head_path)?,
    };
    let mut engine = LocalEngine::new(backend, pointer_head(&head_path)?).with_model(name.clone());
    if let Some(isolation) = isolation {
        engine = engine.with_option_isolation(isolation);
    }

    let listener = TcpListener::bind(&options.listen).await?;
    eprintln!(
        "{name} loaded in {:.1}s, answering on http://{}/v1/systemone{}{}",
        loading.elapsed().as_secs_f64(),
        listener.local_addr()?,
        match options.max_concurrent {
            Some(n) => format!(", {n} at a time"),
            None => String::new(),
        },
        if options.api_key.is_some() {
            ", API key required"
        } else {
            ""
        },
    );

    let mut server = Server::new(engine)
        .model_name(name)
        .api_key(options.api_key);
    if let Some(n) = options.max_concurrent {
        server = server.max_concurrent(n);
    }
    server.serve(listener).await?;
    Ok(())
}

struct Options {
    base: PathBuf,
    checkpoint: Option<PathBuf>,
    head: Option<PathBuf>,
    dtype: Option<DType>,
    device: Device,
    model: Option<String>,
    listen: String,
    api_key: Option<String>,
    max_concurrent: Option<usize>,
    isolation: Option<bool>,
}

const USAGE: &str = "\
usage: serve --base <dir> [--checkpoint <dir>] [options]

the model
  --base <dir>           the base model: config.json and its safetensors
  --checkpoint <dir>     the Kev checkpoint: LoRA adapter, head.pt, tokenizer
  --head <file>          the pointer head, if not <checkpoint>/head.pt
  --dtype f32|f16|bf16   default: f32 on a CPU, bf16 on a GPU, as kev.serve picks it
  --device cpu|metal     where to run; metal needs --features metal (macOS)
  --model <name>         the name requests may ask for besides kev-latest
                         (default: the checkpoint's directory name)
  --option-isolation, --no-option-isolation
                         override the layout head.pt records

the server
  --listen <addr:port>   default 127.0.0.1:8090; 0.0.0.0:8090 in a container
  --api-key <key>        require Authorization: Bearer <key> (default: the
                         RKEV_API_KEY environment variable, else none)
  --max-concurrent <n>   answer n requests at a time, refuse the rest with 503
                         rather than let them wait past a client's timeout
                         (default 4; 0 for no limit)";

fn parse() -> Result<Options, String> {
    let mut options = Options {
        base: PathBuf::new(),
        checkpoint: None,
        head: None,
        dtype: None,
        device: Device::Cpu,
        model: None,
        listen: String::from("127.0.0.1:8090"),
        api_key: std::env::var("RKEV_API_KEY")
            .ok()
            .filter(|key| !key.is_empty()),
        max_concurrent: Some(4),
        isolation: None,
    };
    let mut base = None;

    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| format!("{argument} needs a value"))
        };
        match argument.as_str() {
            "--base" => base = Some(PathBuf::from(value()?)),
            "--checkpoint" => options.checkpoint = Some(PathBuf::from(value()?)),
            "--head" => options.head = Some(PathBuf::from(value()?)),
            "--dtype" => {
                options.dtype = Some(match value()?.as_str() {
                    "f32" | "fp32" | "float32" => DType::F32,
                    "f16" | "fp16" | "float16" => DType::F16,
                    "bf16" | "bfloat16" => DType::BF16,
                    other => return Err(format!("--dtype {other}: f32, f16 or bf16")),
                })
            }
            "--device" => options.device = device(&value()?).map_err(|e| e.to_string())?,
            "--model" => options.model = Some(value()?),
            "--listen" => options.listen = value()?,
            "--api-key" => options.api_key = Some(value()?),
            "--max-concurrent" => {
                let n: usize = value()?
                    .parse()
                    .map_err(|e| format!("--max-concurrent: {e}"))?;
                options.max_concurrent = (n > 0).then_some(n);
            }
            "--option-isolation" => options.isolation = Some(true),
            "--no-option-isolation" => options.isolation = Some(false),
            "-h" | "--help" => return Err(String::from("serve")),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    options.base = base.ok_or("--base is required")?;
    Ok(options)
}
