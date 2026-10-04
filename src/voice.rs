use std::{env, time::Duration};

use http_body_util::{BodyExt, Limited};
use reqwest::{Client, multipart};
use serde_json::{Value, json};
use vercel_runtime::{Request, Response, ResponseBody};

use crate::{
    customer::{error, reply, session_token},
    store::Supabase,
};

const ELEVENLABS_API: &str = "https://api.elevenlabs.io/v1";
const DEFAULT_VOICE: &str = "JBFqnCBsd6RMkjVDRZzb";
const MAX_AUDIO: usize = 2 * 1024 * 1024;

struct ElevenLabs {
    client: Client,
    api: String,
    key: String,
    voice: String,
}

impl ElevenLabs {
    fn from_env() -> Option<Self> {
        let key = env::var("ELEVENLABS_API_KEY").ok()?.trim().to_owned();
        if key.is_empty() {
            return None;
        }
        let voice = env::var("ELEVENLABS_VOICE_ID")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_VOICE.into());
        if !voice
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return None;
        }
        Some(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .ok()?,
            api: ELEVENLABS_API.into(),
            key,
            voice,
        })
    }

    async fn transcribe(&self, audio: Vec<u8>, mime: &str, extension: &str) -> Result<String, ()> {
        let file = multipart::Part::bytes(audio)
            .file_name(format!("message.{extension}"))
            .mime_str(mime)
            .map_err(|_| ())?;
        let form = multipart::Form::new()
            .text("model_id", "scribe_v2")
            .text("tag_audio_events", "false")
            .text("diarize", "false")
            .part("file", file);
        let response = self
            .client
            .post(format!("{}/speech-to-text", self.api))
            .header("xi-api-key", &self.key)
            .multipart(form)
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success() {
            eprintln!(
                "Fizz ElevenLabs transcription failed: HTTP {}.",
                response.status().as_u16()
            );
            return Err(());
        }
        let data: Value = response.json().await.map_err(|_| ())?;
        data.get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .map(str::to_owned)
            .ok_or(())
    }

    async fn speak(&self, text: &str) -> Result<Vec<u8>, ()> {
        let response = self
            .client
            .post(format!(
                "{}/text-to-speech/{}?output_format=mp3_44100_128",
                self.api, self.voice
            ))
            .header("xi-api-key", &self.key)
            .json(&json!({"text":text,"model_id":"eleven_multilingual_v2"}))
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success() {
            eprintln!(
                "Fizz ElevenLabs speech failed: HTTP {}.",
                response.status().as_u16()
            );
            return Err(());
        }
        if !response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("audio/"))
        {
            return Err(());
        }
        let bytes = response.bytes().await.map_err(|_| ())?;
        if bytes.is_empty() || bytes.len() > 4 * 1024 * 1024 {
            return Err(());
        }
        Ok(bytes.to_vec())
    }
}

fn audio_type(content_type: &str) -> Option<(&'static str, &'static str)> {
    match content_type.split(';').next()?.trim() {
        "audio/webm" => Some(("audio/webm", "webm")),
        "audio/ogg" => Some(("audio/ogg", "ogg")),
        "audio/mp4" | "audio/x-m4a" => Some(("audio/mp4", "m4a")),
        "audio/wav" | "audio/x-wav" => Some(("audio/wav", "wav")),
        "audio/mpeg" => Some(("audio/mpeg", "mp3")),
        _ => None,
    }
}

pub async fn handle(request: Request) -> Response<ResponseBody> {
    if !matches!(request.method().as_str(), "GET" | "POST") {
        return error(405, "method_not_allowed", "Use GET or POST.");
    }
    let Some(token) = session_token(&request) else {
        return error(401, "unauthorized", "Sign in to use voice chat.");
    };
    let Ok(store) = Supabase::from_env() else {
        return error(
            503,
            "database_unavailable",
            "Voice chat is temporarily unavailable.",
        );
    };
    match store
        .rpc("fizz_current_customer", json!({"p_token":token}))
        .await
    {
        Ok(value) if value.get("username").is_some() => {}
        Ok(_) => return error(401, "unauthorized", "Sign in to use voice chat."),
        Err(_) => {
            return error(
                503,
                "database_unavailable",
                "Voice chat is temporarily unavailable.",
            );
        }
    }
    let provider = ElevenLabs::from_env();
    if request.method() == "GET" {
        return reply(200, json!({"available":provider.is_some()}), None);
    }
    let Some(provider) = provider else {
        return error(
            503,
            "voice_unavailable",
            "Voice chat isn't available yet. You can still type to Fizz.",
        );
    };
    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let is_json = content_type
        .split(';')
        .next()
        .is_some_and(|mime| mime.trim() == "application/json");
    let audio = audio_type(&content_type);
    if !is_json && audio.is_none() {
        return error(
            415,
            "unsupported_media_type",
            "Send an audio recording or a speech request.",
        );
    }
    let limit = if is_json { 16384 } else { MAX_AUDIO };
    let bytes = match Limited::new(request.into_body(), limit).collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => {
            return error(
                413,
                "too_large",
                "Keep voice messages under 30 seconds and 2 MB.",
            );
        }
    };
    if is_json {
        let Ok(data): Result<Value, _> = serde_json::from_slice(&bytes) else {
            return error(400, "invalid_request", "Send valid JSON.");
        };
        let Some(text) = data
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty() && text.len() <= 3000)
        else {
            return error(
                400,
                "invalid_request",
                "Speech must contain 1–3,000 characters.",
            );
        };
        return match provider.speak(text).await {
            Ok(audio) => Response::builder()
                .status(200)
                .header("content-type", "audio/mpeg")
                .header("cache-control", "no-store")
                .header("x-content-type-options", "nosniff")
                .body(ResponseBody::from(audio))
                .expect("valid audio response"),
            Err(_) => error(
                503,
                "voice_unavailable",
                "I couldn't speak that reply. You can read it in chat.",
            ),
        };
    }
    if bytes.is_empty() {
        return error(400, "invalid_audio", "Record a voice message first.");
    }
    let (mime, extension) = audio.expect("validated audio content type");
    match provider.transcribe(bytes.to_vec(), mime, extension).await {
        Ok(text) if !text.is_empty() && text.len() <= 1000 => {
            reply(200, json!({"text":text}), None)
        }
        Ok(text) if text.is_empty() => error(
            400,
            "no_speech",
            "I didn't hear any words. Please try again.",
        ),
        Ok(_) => error(
            400,
            "too_long",
            "Try a shorter message, up to 1,000 characters.",
        ),
        Err(_) => error(
            503,
            "voice_unavailable",
            "I couldn't transcribe that recording. Try again or type your message.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_recording_formats_include_browser_codecs() {
        assert_eq!(
            audio_type("audio/webm;codecs=opus"),
            Some(("audio/webm", "webm"))
        );
        assert_eq!(audio_type("audio/mp4"), Some(("audio/mp4", "m4a")));
        assert_eq!(audio_type("audio/mpeg"), Some(("audio/mpeg", "mp3")));
        assert_eq!(audio_type("text/html"), None);
        assert_eq!(audio_type("application/json"), None);
    }
}
