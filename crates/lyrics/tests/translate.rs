//! Translation over a fake transport: request shape, both reply formats, line-count
//! fitting, and the typed refusal when no key is configured.

use std::sync::Arc;

use ml_lyrics::{
    parse_translation_content, FakeTransport, TranslationClient, TranslationConfig,
    TranslationEndpoint,
};

#[tokio::test]
async fn deepl_request_carries_the_key_and_language() {
    let http = FakeTransport::new([(
        "https://api.deepl.com".to_string(),
        r#"{"translations":[{"text":"hallo"},{"text":"welt"}]}"#.to_string(),
    )]);
    let transport = http.clone();
    let client = TranslationClient::new(
        http,
        TranslationConfig::new(TranslationEndpoint::DeepL, "test-key", "de"),
    );
    let lines = client
        .translate_lines(&["hello".into(), "world".into()])
        .await
        .unwrap();
    assert_eq!(lines, ["hallo", "welt"]);

    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    let (url, headers, body) = &requests[0];
    assert_eq!(url, "https://api.deepl.com/v2/translate");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value == "DeepL-Auth-Key test-key"),
        "{headers:?}"
    );
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["target_lang"], "DE");
    assert_eq!(json["text"][0], "hello");
    assert_eq!(json["preserve_formatting"], true);
    // The key never appears in the URL.
    assert!(!url.contains("test-key"));
}

/// DeepL free keys end in `:fx`; sending them to the pro host 403s.
#[tokio::test]
async fn deepl_free_keys_use_the_free_host() {
    let http = FakeTransport::new([(
        "https://api-free.deepl.com".to_string(),
        r#"{"translations":[{"text":"hallo"}]}"#.to_string(),
    )]);
    let transport = http.clone();
    let client = TranslationClient::new(
        http,
        TranslationConfig::new(TranslationEndpoint::DeepL, "abc:fx", "de"),
    );
    client.translate_lines(&["hello".into()]).await.unwrap();
    assert_eq!(
        transport.requests()[0].0,
        "https://api-free.deepl.com/v2/translate"
    );
}

#[tokio::test]
async fn openai_compatible_request_and_reply() {
    let http = FakeTransport::new([(
        "https://openrouter.ai".to_string(),
        r#"{"choices":[{"message":{"content":"```json\n{\"lines\":[\"hola\",\"mundo\"]}\n```"}}]}"#
            .to_string(),
    )]);
    let transport = http.clone();
    let client = TranslationClient::new(
        http,
        TranslationConfig::new(
            TranslationEndpoint::OpenAiCompatible {
                url: "https://openrouter.ai/api/v1/chat/completions".into(),
            },
            "sk-test",
            "es",
        )
        .with_model("some/model"),
    );
    let lines = client
        .translate_lines(&["hello".into(), "world".into()])
        .await
        .unwrap();
    assert_eq!(lines, ["hola", "mundo"]);

    let (url, headers, body) = transport.requests().remove(0);
    assert_eq!(url, "https://openrouter.ai/api/v1/chat/completions");
    assert!(headers
        .iter()
        .any(|(name, value)| name == "Authorization" && value == "Bearer sk-test"));
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["model"], "some/model");
    assert!(json["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("EXACTLY 2 strings"));
}

/// Missing key: typed `Unavailable`, and the transport is never asked.
#[tokio::test]
async fn no_key_refuses_without_a_request() {
    for endpoint in [
        TranslationEndpoint::DeepL,
        TranslationEndpoint::OpenAiCompatible {
            url: "https://example.invalid/v1/chat/completions".into(),
        },
    ] {
        let http = FakeTransport::default();
        let transport = Arc::new(http);
        let client = TranslationClient::new(
            transport.clone(),
            TranslationConfig::new(endpoint, "   ", "de"),
        );
        let error = client.translate_lines(&["hello".into()]).await.unwrap_err();
        assert!(
            matches!(error, ml_core::MlError::Unavailable(_)),
            "{error:?}"
        );
        assert!(transport.requests().is_empty());
    }
}

#[tokio::test]
async fn short_and_long_replies_fit_the_input() {
    for (reply, expected) in [
        (
            r#"{"translations":[{"text":"eins"}]}"#,
            vec!["eins".to_string(), String::new(), String::new()],
        ),
        (
            r#"{"translations":[{"text":"eins"},{"text":"zwei"},{"text":"drei"},{"text":"vier"}]}"#,
            vec!["eins".to_string(), "zwei".to_string(), "drei".to_string()],
        ),
    ] {
        let http = FakeTransport::new([("https://api.deepl.com".to_string(), reply.to_string())]);
        let client = TranslationClient::new(
            http,
            TranslationConfig::new(TranslationEndpoint::DeepL, "k", "de"),
        );
        let lines = client
            .translate_lines(&["one".into(), "two".into(), "three".into()])
            .await
            .unwrap();
        assert_eq!(lines, expected, "{reply}");
    }
}

#[tokio::test]
async fn empty_input_makes_no_request() {
    let http = FakeTransport::new([(
        "https://api.deepl.com".to_string(),
        r#"{"translations":[]}"#.to_string(),
    )]);
    let transport = http.clone();
    let client = TranslationClient::new(
        http,
        TranslationConfig::new(TranslationEndpoint::DeepL, "k", "de"),
    );
    let lines = client
        .translate_lines(&["".into(), "  ".into()])
        .await
        .unwrap();
    assert_eq!(lines, ["", ""]);
    // Already empty: there is nothing to send.
    assert!(transport.requests().is_empty());
}

/// A blank line between lyrics keeps its slot but is not sent: DeepL rejects empty
/// `text` entries, and one blank line must not shift the whole timeline.
#[tokio::test]
async fn blank_lines_keep_their_slot_but_are_not_sent() {
    let http = FakeTransport::new([(
        "https://api.deepl.com".to_string(),
        r#"{"translations":[{"text":"eins"},{"text":"drei"}]}"#.to_string(),
    )]);
    let transport = http.clone();
    let client = TranslationClient::new(
        http,
        TranslationConfig::new(TranslationEndpoint::DeepL, "k", "de"),
    );
    let lines = client
        .translate_lines(&["one".into(), "   ".into(), "three".into()])
        .await
        .unwrap();
    assert_eq!(lines, ["eins", "", "drei"]);
    let body = &transport.requests().remove(0).2;
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["text"].as_array().unwrap().len(), 2, "{body}");
}

#[tokio::test]
async fn document_translation_keeps_timings() {
    let http = FakeTransport::new([(
        "https://api.deepl.com".to_string(),
        r#"{"translations":[{"text":"hallo"},{"text":"welt"}]}"#.to_string(),
    )]);
    let client = TranslationClient::new(
        http,
        TranslationConfig::new(TranslationEndpoint::DeepL, "k", "de"),
    );
    let mut document = ml_core::LyricsDocument {
        provider: "test".into(),
        lines: ["hello", "world"]
            .iter()
            .enumerate()
            .map(|(i, text)| ml_core::LyricsLine {
                start_ms: i as u64 * 1_000,
                end_ms: Some(i as u64 * 1_000 + 900),
                text: text.to_string(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let lines = client.translate_document(&document).await.unwrap();
    document.translated = Some(lines);
    let translated = document.translated.unwrap();
    assert_eq!(translated[0].text, "hallo");
    assert_eq!(translated[0].start_ms, 0);
    assert_eq!(translated[1].end_ms, Some(1_900));
}

#[test]
fn reply_parsing_variants() {
    assert_eq!(
        parse_translation_content(r#"{"lines":["a","b"]}"#),
        ["a", "b"]
    );
    assert_eq!(parse_translation_content(r#"["a","b"]"#), ["a", "b"]);
    assert_eq!(parse_translation_content("```json\n[\"a\"]\n```"), ["a"]);
    assert_eq!(parse_translation_content("\"quoted\""), ["quoted"]);
    assert_eq!(parse_translation_content("a\nb"), ["a", "b"]);
    assert!(parse_translation_content("").is_empty());
}
