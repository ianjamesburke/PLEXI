//! WebSocket session for gpt-realtime-2 (#816).
//!
//! Runs a live voice session: mic → PCM16 24kHz → gpt-realtime-2 → audio out
//! + text transcript rendered to the PTY. Tool calls are executed as plexi
//! subprocess calls using PLEXI_SOCKET (auto-injected since agent runs in a pane).

use std::io::Write as _;
use std::thread;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const MODEL: &str = "gpt-realtime-2";

fn system_prompt() -> String {
    "You are a Plexi workspace assistant running inside a Plexi terminal pane. \
    You can control the workspace by calling the available tools, which map directly \
    to `plexi` CLI subcommands. Keep responses concise. When you call a tool, it runs \
    as a subprocess in this pane's environment with PLEXI_SOCKET set, so host commands \
    will reach the running Plexi instance."
        .to_string()
}

fn get_tools() -> Vec<Value> {
    use clap::CommandFactory;
    let cmd = crate::cli_args::Cli::command();
    let mut tools = Vec::new();
    for sub in cmd.get_subcommands() {
        crate::agent::schema::collect_tools_pub(sub, "", &mut tools);
    }
    tools.retain(|t| {
        let name = t["name"].as_str().unwrap_or("");
        !name.starts_with("agent_")
    });
    tools
}

fn execute_tool(name: &str, args: &Value) -> String {
    // Convert tool name back to CLI args: "pane_open" → ["pane", "open", ...]
    let parts: Vec<&str> = name.split('_').collect();
    let mut cmd_args: Vec<String> = parts.iter().map(|s| s.to_string()).collect();

    // Append positional and flag args from the JSON object
    if let Some(obj) = args.as_object() {
        for (k, v) in obj {
            match v {
                Value::Bool(true) => cmd_args.push(format!("--{}", k)),
                Value::Bool(false) => {}
                Value::String(s) => {
                    cmd_args.push(s.clone());
                }
                Value::Number(n) => cmd_args.push(n.to_string()),
                _ => {}
            }
        }
    }

    log::info!("agent:tool_call: {} args={:?}", name, cmd_args);

    let binary =
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("plexi"));

    match std::process::Command::new(&binary).args(&cmd_args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if !stderr.is_empty() {
                format!("{stdout}\n{stderr}")
            } else {
                stdout
            }
        }
        Err(e) => {
            log::warn!("agent:tool_exec_failed: {} — {e}", name);
            format!("error: {e}")
        }
    }
}

/// Capture mic audio, downsample to 24kHz PCM16 mono, and send chunks to `tx`.
/// Always outputs at 24kHz regardless of device sample rate.
fn setup_audio_capture(tx: std::sync::mpsc::Sender<Vec<u8>>) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no default input device".to_string())?;

    let supported = device
        .default_input_config()
        .map_err(|e| format!("no default input config: {e}"))?;

    let device_rate = supported.sample_rate();
    let ch = supported.channels();
    log::info!("agent:audio: device rate={device_rate} channels={ch} format={:?}", supported.sample_format());

    let config: cpal::StreamConfig = supported.into();
    let ratio = device_rate as f64 / 24_000f64;

    let stream = device
        .build_input_stream(
            &config,
            move |data: &[f32], _| {
                // Mix to mono then downsample to 24kHz via nearest-neighbour decimation.
                let mono: Vec<f32> = data
                    .chunks(ch as usize)
                    .map(|frame| frame.iter().copied().sum::<f32>() / ch as f32)
                    .collect();

                let out_len = (mono.len() as f64 / ratio) as usize;
                let pcm: Vec<u8> = (0..out_len)
                    .flat_map(|i| {
                        let src = (i as f64 * ratio) as usize;
                        let s = mono.get(src).copied().unwrap_or(0.0).clamp(-1.0, 1.0);
                        let i16_val = (s * 32767.0) as i16;
                        i16_val.to_le_bytes()
                    })
                    .collect();

                if !pcm.is_empty() {
                    let _ = tx.send(pcm);
                }
            },
            |e| log::error!("agent:audio_capture_error: {e}"),
            None,
        )
        .map_err(|e| format!("failed to build input stream: {e}"))?;

    stream.play().map_err(|e| format!("failed to start stream: {e}"))?;
    Ok(stream)
}

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    Message,
>;

async fn handle_server_event(
    event: &Value,
    ws_tx: &mut WsSink,
    pending_tool: &mut Option<(String, String, String)>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let event_type = event["type"].as_str().unwrap_or("");

    match event_type {
        "session.created" | "session.updated" => {
            log::info!("agent:session: {event_type}");
        }

        "conversation.item.input_audio_transcription.delta" => {
            if let Some(delta) = event["delta"].as_str() {
                print!("\x1b[2m{delta}\x1b[0m");
                let _ = std::io::stdout().flush();
            }
        }

        "response.text.delta" => {
            if let Some(delta) = event["delta"].as_str() {
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
        }

        "response.text.done" => {
            println!();
        }

        "response.function_call_arguments.start" => {
            let call_id = event["call_id"].as_str().unwrap_or("").to_string();
            let name = event["name"].as_str().unwrap_or("").to_string();
            log::info!("agent:tool_start: call_id={call_id} name={name}");
            print!("\n\x1b[33m→ {name}\x1b[0m");
            let _ = std::io::stdout().flush();
            *pending_tool = Some((call_id, name, String::new()));
        }

        "response.function_call_arguments.delta" => {
            if let Some((_, _, ref mut args)) = pending_tool {
                if let Some(delta) = event["delta"].as_str() {
                    args.push_str(delta);
                }
            }
        }

        "response.function_call_arguments.done" => {
            if let Some((call_id, name, args)) = pending_tool.take() {
                let args_value: Value =
                    serde_json::from_str(&args).unwrap_or(json!({}));
                println!(
                    " {}",
                    serde_json::to_string(&args_value).unwrap_or_default()
                );

                log::info!("agent:tool_execute: name={name} args={args}");
                let result = execute_tool(&name, &args_value);
                log::info!(
                    "agent:tool_result: name={name} result_len={}",
                    result.len()
                );

                // Submit tool result back to the session
                let response_event = json!({
                    "type": "conversation.item.create",
                    "item": {
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": result
                    }
                });
                ws_tx
                    .send(Message::Text(response_event.to_string()))
                    .await?;

                // Trigger the model to continue
                let continue_event = json!({ "type": "response.create" });
                ws_tx
                    .send(Message::Text(continue_event.to_string()))
                    .await?;
            }
        }

        // GA API delivers completed tool calls via response.done output array.
        "response.done" => {
            if let Some(outputs) = event["response"]["output"].as_array() {
                for item in outputs {
                    if item["type"].as_str() == Some("function_call") {
                        let call_id = item["call_id"].as_str().unwrap_or("").to_string();
                        let name = item["name"].as_str().unwrap_or("").to_string();
                        let args_str = item["arguments"].as_str().unwrap_or("{}");
                        let args_value: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

                        println!("\n\x1b[33m→ {name} {}\x1b[0m", serde_json::to_string(&args_value).unwrap_or_default());
                        log::info!("agent:tool_execute: name={name} call_id={call_id}");

                        let result = execute_tool(&name, &args_value);
                        log::info!("agent:tool_result: name={name} result_len={}", result.len());

                        let output_event = json!({
                            "type": "conversation.item.create",
                            "item": {
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": result
                            }
                        });
                        ws_tx.send(Message::Text(output_event.to_string())).await?;
                        ws_tx.send(Message::Text(json!({ "type": "response.create" }).to_string())).await?;
                    }
                }
            }
        }

        "error" => {
            let msg = event["error"]["message"]
                .as_str()
                .unwrap_or("unknown error");
            log::error!("agent:api_error: {msg}");
            eprintln!("\x1b[31m[agent] API error: {msg}\x1b[0m");
        }

        _ => {
            // Log all unhandled events at info so we can discover the GA API event names.
            log::info!("agent:event:unhandled type={event_type} keys={}",
                event.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>().join(",")).unwrap_or_default());
        }
    }

    Ok(())
}

async fn run_session(
    api_key: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("wss://api.openai.com/v1/realtime?model={MODEL}");

    // Build from the URL so tungstenite generates Sec-WebSocket-Key automatically,
    // then add auth headers on top.
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("failed to build request: {e}"))?;
    let headers = request.headers_mut();
    headers.insert(
        "Authorization",
        format!("Bearer {api_key}").parse().map_err(|e| format!("bad auth header: {e}"))?,
    );
    // gpt-realtime-2 is the GA API — no OpenAI-Beta header needed.

    log::info!("agent:session: connecting to {url}");
    eprintln!("\x1b[2m[agent] Connecting to gpt-realtime-2...\x1b[0m");

    let (ws_stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| format!("WebSocket connect failed: {e}"))?;

    log::info!("agent:session: connected");
    eprintln!(
        "\x1b[32m[agent] Connected. Speak now — press Ctrl+C to end session.\x1b[0m"
    );

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let tools = get_tools();
    log::info!("agent:session: {} tools available", tools.len());

    // Start audio capture. Always outputs 24kHz PCM16 after downsampling.
    let (audio_tx, mut audio_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (sync_tx, sync_rx) = std::sync::mpsc::channel::<Vec<u8>>();

    let audio_tx_bridge = audio_tx.clone();
    thread::spawn(move || {
        while let Ok(chunk) = sync_rx.recv() {
            if audio_tx_bridge.send(chunk).is_err() {
                break;
            }
        }
    });

    // cpal::Stream is not Send — own it in a dedicated thread.
    thread::spawn(move || {
        match setup_audio_capture(sync_tx) {
            Ok(_stream) => loop {
                thread::sleep(std::time::Duration::from_secs(3600));
            },
            Err(e) => {
                log::error!("agent:audio_setup_failed: {e}");
                eprintln!("\x1b[31m[agent] Audio capture failed: {e}\x1b[0m");
            }
        }
    });

    // session.update — GA API schema: audio config is nested under audio.input / audio.output.
    // tools inline registration not supported in gpt-realtime-2; set instructions only for now.
    let session_update = json!({
        "type": "session.update",
        "session": {
            "type": "realtime",
            "instructions": system_prompt(),
            "output_modalities": ["audio"],
            "audio": {
                "input": {
                    "format": { "type": "audio/pcm", "rate": 24000 },
                    "turn_detection": {
                        "type": "server_vad",
                        "threshold": 0.5,
                        "prefix_padding_ms": 300,
                        "silence_duration_ms": 500,
                        "create_response": true,
                        "interrupt_response": true
                    }
                },
                "output": {
                    "voice": "alloy"
                }
            }
        }
    });

    ws_tx
        .send(Message::Text(session_update.to_string()))
        .await?;
    log::info!("agent:session: session.update sent");

    // Pending tool call accumulator: (call_id, name, args_so_far)
    let mut pending_tool: Option<(String, String, String)> = None;

    // Event loop
    loop {
        tokio::select! {
            // Forward audio chunks to the API
            Some(chunk) = audio_rx.recv() => {
                let b64 = BASE64.encode(&chunk);
                let event = json!({
                    "type": "input_audio_buffer.append",
                    "audio": b64
                });
                ws_tx.send(Message::Text(event.to_string())).await?;
            }

            // Handle server events
            msg = ws_rx.next() => {
                match msg {
                    None => {
                        log::info!("agent:session: WebSocket closed by server");
                        break;
                    }
                    Some(Err(e)) => {
                        log::error!("agent:session: WebSocket error: {e}");
                        return Err(Box::new(e));
                    }
                    Some(Ok(Message::Text(text))) => {
                        let event: Value =
                            serde_json::from_str(&text).unwrap_or(Value::Null);
                        handle_server_event(&event, &mut ws_tx, &mut pending_tool)
                            .await?;
                    }
                    Some(Ok(Message::Close(_))) => {
                        log::info!("agent:session: received close frame");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

pub fn run() -> i32 {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!("error: OPENAI_API_KEY is not set.");
            eprintln!("Set it with: plexi secret set --inject OPENAI_API_KEY");
            eprintln!("Then restart this pane.");
            return 1;
        }
    };

    log::info!("agent:start: launching session");

    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: failed to create async runtime: {e}");
            return 1;
        }
    };

    match rt.block_on(run_session(api_key)) {
        Ok(()) => {
            log::info!("agent:session: ended cleanly");
            0
        }
        Err(e) => {
            log::error!("agent:session: error: {e}");
            eprintln!("\x1b[31m[agent] Session error: {e}\x1b[0m");
            1
        }
    }
}
