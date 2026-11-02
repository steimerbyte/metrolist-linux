//! Machine translation over a **user-supplied** API key. Nothing is hardcoded and no
//! undocumented endpoint is called.
//!
//! Two transports are supported, both with documented public APIs:
//!
//! * DeepL (`/v2/translate` at `api.deepl.com`, or `api-free.deepl.com` for `:fx` keys)
//! * any OpenAI-compatible chat-completions endpoint (OpenRouter, OpenAI, a local server)
//!
//! Without a key every call answers [`MlError::Unavailable`] — there is no free tier
//! fallback, because the keyless public endpoints (Google's unofficial
//! `translate.googleapis.com`, public LibreTranslate mirrors) are either undocumented or
//! rate-limited hosting with no terms that permit an app to use them.
//!
//! Line handling matches upstream `LyricsTranslationHelper`: one request for the whole
//! document, a JSON `{"lines": [...]}` reply, missing lines padded with `""` and extras
//! dropped, so the result lines up with the original timeline.

use std::sync::Arc;

use ml_core::{LyricsLine, MlError, MlResult};
use serde::Deserialize;

use crate::providers::{HttpTransport, RecordedRequest};

/// Where a translation request goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationEndpoint {
    /// DeepL. `api-free.deepl.com` is used when the key ends in `:fx`.
    DeepL,
    /// An OpenAI-compatible `POST {url}` returning `choices[0].message.content`.
    OpenAiCompatible { url: String },
}

/// A configured translation service: endpoint, user key, target language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslationConfig {
    pub endpoint: TranslationEndpoint,
    pub api_key: String,
    pub model: Option<String>,
    /// Target language as the endpoint understands it (`en`, `DE`, `zh-Hans`, …).
    pub target_language: String,
}

impl TranslationConfig {
    pub fn new(
        endpoint: TranslationEndpoint,
        api_key: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            endpoint,
            api_key: api_key.into(),
            model: None,
            target_language: target.into(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Read a key from the environment; `None` means "not configured", never a default.
    ///
    /// * `METROLIST_DEEPL_KEY` — DeepL
    /// * `METROLIST_OPENAI_KEY` + `METROLIST_OPENAI_URL` (optional `METROLIST_OPENAI_MODEL`)
    ///
    /// A key in the process environment keeps it out of the settings file and the repo;
    /// the UI should offer the same pair as a settings field.
    pub fn from_env(target: impl Into<String>) -> Option<Self> {
        let target = target.into();
        if let Ok(key) = std::env::var("METROLIST_DEEPL_KEY") {
            if !key.trim().is_empty() {
                return Some(Self::new(TranslationEndpoint::DeepL, key.trim(), target));
            }
        }
        let key = std::env::var("METROLIST_OPENAI_KEY").ok()?;
        let url = std::env::var("METROLIST_OPENAI_URL").ok()?;
        if key.trim().is_empty() || url.trim().is_empty() {
            return None;
        }
        let config = Self::new(
            TranslationEndpoint::OpenAiCompatible { url },
            key.trim(),
            target,
        );
        Some(match std::env::var("METROLIST_OPENAI_MODEL") {
            Ok(model) if !model.trim().is_empty() => config.with_model(model.trim()),
            _ => config,
        })
    }

    /// Percent-safe presence check used to decide between a request and a typed refusal.
    pub fn is_configured(&self) -> bool {
        !self.api_key.trim().is_empty()
    }
}

/// Translate whole lines, keeping the line count.
#[derive(Clone)]
pub struct TranslationClient {
    http: Arc<dyn HttpTransport>,
    config: TranslationConfig,
}

impl std::fmt::Debug for TranslationClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranslationClient")
            .field("endpoint", &self.config.endpoint)
            .field("target_language", &self.config.target_language)
            .finish_non_exhaustive()
    }
}

impl TranslationClient {
    pub fn new(http: Arc<dyn HttpTransport>, config: TranslationConfig) -> Self {
        Self { http, config }
    }

    pub fn config(&self) -> &TranslationConfig {
        &self.config
    }

    /// Translate `lines`; the result has exactly `lines.len()` entries.
    ///
    /// Fails with [`MlError::Unavailable`] when no key is configured, and with
    /// [`MlError::Http`]/[`MlError::Parse`] when the service refuses or answers junk.
    pub async fn translate_lines(&self, lines: &[String]) -> MlResult<Vec<String>> {
        if !self.config.is_configured() {
            return Err(MlError::Unavailable(
                "translation needs a DeepL or OpenAI-compatible API key".into(),
            ));
        }
        // Blank lines are not sent (DeepL rejects empty `text` entries) but keep their
        // slot, so the reply still lines up with the timeline.
        let kept: Vec<usize> = (0..lines.len())
            .filter(|&i| !lines[i].trim().is_empty())
            .collect();
        if kept.is_empty() {
            return Ok(vec![String::new(); lines.len()]);
        }
        let payload: Vec<String> = kept.iter().map(|&i| lines[i].clone()).collect();

        let (url, headers, body) = self.build_request(&payload)?;
        let response = self.http.post(&url, headers, body).await?;
        let translated = match self.config.endpoint {
            TranslationEndpoint::DeepL => parse_deepl(&response)?,
            TranslationEndpoint::OpenAiCompatible { .. } => parse_openai(&response)?,
        };
        let translated = fit_to_line_count(translated, kept.len());

        let mut result = vec![String::new(); lines.len()];
        for (slot, text) in kept.into_iter().zip(translated) {
            result[slot] = text;
        }
        Ok(result)
    }

    /// Translate a document's lines into `doc.translated`, keeping every timing.
    pub async fn translate_document(
        &self,
        doc: &ml_core::LyricsDocument,
    ) -> MlResult<Vec<LyricsLine>> {
        let texts: Vec<String> = doc.lines.iter().map(|line| line.text.clone()).collect();
        let translated = self.translate_lines(&texts).await?;
        Ok(doc
            .lines
            .iter()
            .zip(translated)
            .map(|(line, text)| LyricsLine {
                text,
                ..line.clone()
            })
            .collect())
    }

    fn build_request(&self, lines: &[String]) -> MlResult<RecordedRequest> {
        let json = |value: serde_json::Value| value.to_string();
        match &self.config.endpoint {
            TranslationEndpoint::DeepL => {
                let base = if self.config.api_key.ends_with(":fx") {
                    "https://api-free.deepl.com/v2/translate"
                } else {
                    "https://api.deepl.com/v2/translate"
                };
                let body = json(serde_json::json!({
                    "text": lines,
                    "target_lang": deepl_language(&self.config.target_language),
                    "preserve_formatting": true,
                }));
                let headers = vec![(
                    "Authorization".to_string(),
                    format!("DeepL-Auth-Key {}", self.config.api_key),
                )];
                Ok((base.to_string(), headers, body))
            }
            TranslationEndpoint::OpenAiCompatible { url } => {
                let count = lines.len();
                let system = format!(
                    "You are a precise lyrics translation assistant. Output ONLY a JSON object \
                     {{\"lines\": [\"line1\", \"line2\"]}} with EXACTLY {count} strings, empty input \
                     lines preserved as \"\"."
                );
                let user = format!(
                    "Translate the following {count} lines to {}. Keep punctuation and \
                     line-by-line structure.\n\n{}",
                    self.config.target_language,
                    lines.join("\n")
                );
                let mut payload = serde_json::json!({
                    "messages": [
                        {"role": "system", "content": system},
                        {"role": "user", "content": user},
                    ],
                });
                if let Some(model) = &self.config.model {
                    payload["model"] = serde_json::Value::String(model.clone());
                }
                let headers = vec![(
                    "Authorization".to_string(),
                    format!("Bearer {}", self.config.api_key),
                )];
                Ok((url.clone(), headers, json(payload)))
            }
        }
    }
}

/// DeepL wants uppercase language codes with a region for English/Portuguese.
fn deepl_language(target: &str) -> String {
    match target.to_lowercase().as_str() {
        "zh" | "zh-cn" | "zh-hans" | "zh-tw" | "zh-hant" => "ZH".to_string(),
        "en" => "EN-US".to_string(),
        "en-gb" => "EN-GB".to_string(),
        "pt" => "PT-PT".to_string(),
        "pt-br" => "PT-BR".to_string(),
        other => other.to_uppercase(),
    }
}

#[derive(Deserialize)]
struct DeepLResponse {
    #[serde(default)]
    translations: Vec<DeepLText>,
}

#[derive(Deserialize)]
struct DeepLText {
    #[serde(default)]
    text: String,
}

fn parse_deepl(body: &str) -> MlResult<Vec<String>> {
    let parsed: DeepLResponse =
        serde_json::from_str(body).map_err(|e| MlError::Parse(format!("deepl: {e}")))?;
    Ok(parsed.translations.into_iter().map(|t| t.text).collect())
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    message: Message,
}

#[derive(Default, Deserialize)]
struct Message {
    #[serde(default)]
    content: String,
}

fn parse_openai(body: &str) -> MlResult<Vec<String>> {
    let parsed: ChatResponse =
        serde_json::from_str(body).map_err(|e| MlError::Parse(format!("translation: {e}")))?;
    let content = parsed
        .choices
        .first()
        .map(|choice| choice.message.content.as_str())
        .unwrap_or_default();
    if content.trim().is_empty() {
        return Err(MlError::Parse("translation: empty reply".into()));
    }
    Ok(parse_translation_content(content))
}

/// Lenient reply parser, ported from upstream `parseTranslationContent`: accept a
/// fenced block, `{"lines": [...]}`, a bare array, or one line per reply line.
pub fn parse_translation_content(content: &str) -> Vec<String> {
    let cleaned = content
        .replace("```json", "")
        .replace("```", "")
        .trim()
        .to_string();
    for candidate in [content.trim().to_string(), cleaned.clone()] {
        if let Some(lines) = lines_from_json(&candidate) {
            return lines;
        }
    }
    if let Some(start) = cleaned.find('[') {
        if let Some(end) = cleaned.rfind(']') {
            if end > start {
                let bracketed = format!("[{}]", &cleaned[start + 1..end]);
                if let Some(lines) = lines_from_json(&bracketed) {
                    return lines;
                }
            }
        }
    }
    cleaned
        .lines()
        .map(|line| line.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

fn lines_from_json(text: &str) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let array = match &value {
        serde_json::Value::Object(map) => map.get("lines")?.as_array()?,
        serde_json::Value::Array(items) => items,
        _ => return None,
    };
    Some(
        array
            .iter()
            .map(|item| match item {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect(),
    )
}

/// Exactly `count` entries: extras dropped, missing lines padded with `""`.
fn fit_to_line_count(mut lines: Vec<String>, count: usize) -> Vec<String> {
    lines.resize(count, String::new());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepl_language_codes() {
        assert_eq!(deepl_language("en"), "EN-US");
        assert_eq!(deepl_language("de"), "DE");
        assert_eq!(deepl_language("zh-CN"), "ZH");
    }

    #[test]
    fn replies_parse_leniently() {
        assert_eq!(
            parse_translation_content(r#"{"lines":["uno",""]}"#),
            ["uno", ""]
        );
        assert_eq!(
            parse_translation_content("```json\n[\"uno\",\"dos\"]\n```"),
            ["uno", "dos"]
        );
        assert_eq!(parse_translation_content("one\ntwo"), ["one", "two"]);
        assert_eq!(parse_translation_content("\"quoted\""), ["quoted"]);
    }

    #[test]
    fn short_replies_are_padded_and_long_ones_truncated() {
        assert_eq!(fit_to_line_count(vec!["a".into()], 3), ["a", "", ""]);
        assert_eq!(fit_to_line_count(vec!["a".into(), "b".into()], 1), ["a"]);
        assert_eq!(fit_to_line_count(Vec::new(), 2), ["", ""]);
    }

    #[tokio::test]
    async fn unconfigured_client_refuses_without_a_request() {
        // The transport is irrelevant: no key means no request.
        let client = TranslationClient::new(
            Arc::new(crate::providers::FakeTransport::default()),
            TranslationConfig::new(TranslationEndpoint::DeepL, "", "de"),
        );
        let error = client.translate_lines(&["hello".into()]).await;
        assert!(matches!(error, Err(MlError::Unavailable(_))), "{error:?}");
    }
}
