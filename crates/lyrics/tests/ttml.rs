//! Port of upstream `TTMLParserTest` (14 cases). Assertions are on the LRC string the
//! app consumes, exactly like the Kotlin test, because that is the contract.

use ml_core::LyricsLine;
use ml_lyrics::{parse_ttml, to_lrc};

fn lrc(ttml: &str) -> String {
    to_lrc(&parse_ttml(ttml).expect("valid TTML").lines)
}

fn lines(ttml: &str) -> Vec<LyricsLine> {
    parse_ttml(ttml).expect("valid TTML").lines
}

#[test]
fn v1000_agent_mapping() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata">
          <head>
            <metadata>
              <ttm:agent xml:id="v1000" type="group"/>
              <ttm:agent xml:id="v1" type="person"/>
            </metadata>
          </head>
          <body>
            <div>
              <p begin="00:01.500" ttm:agent="v1000">
                <span>(Group vocal)</span>
              </p>
              <p begin="00:02.000" ttm:agent="v1">
                <span>Main vocal</span>
              </p>
            </div>
          </body>
        </tt>
    "#;
    let lrc = lrc(ttml);
    assert!(lrc.contains("{agent:v2}(Group vocal)"), "{lrc}");
    assert!(lrc.contains("{agent:v1}Main vocal"), "{lrc}");
}

#[test]
fn time_formats() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml">
          <body>
            <div>
              <p begin="1.5s"><span>1.5 seconds</span></p>
              <p begin="2000ms"><span>2000 milliseconds</span></p>
              <p begin="00:03.50"><span>Standard format</span></p>
            </div>
          </body>
        </tt>
    "#;
    let lrc = lrc(ttml);
    let lines: Vec<&str> = lrc.trim().lines().collect();
    assert!(lines[0].starts_with("[00:01.50]"), "{lrc}");
    assert!(lines[1].starts_with("[00:02.00]"), "{lrc}");
    assert!(lines[2].starts_with("[00:03.50]"), "{lrc}");
}

#[test]
fn role_x_bg_maps_to_bg_tag() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata">
          <body>
            <div>
              <p begin="00:05.000"><span ttm:role="x-bg">Background role</span></p>
            </div>
          </body>
        </tt>
    "#;
    assert!(lrc(ttml).contains("{bg}Background role"));
}

#[test]
fn word_level_sync() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml">
          <body>
            <div>
              <p begin="00:10.000">
                <span begin="00:10.000" end="00:10.500">Hello</span>
                <span begin="00:10.600" end="00:11.000">world</span>
              </p>
            </div>
          </body>
        </tt>
    "#;
    assert!(
        lrc(ttml).contains("<Hello:10.0:10.5|world:10.6:11.0>"),
        "{}",
        lrc(ttml)
    );
}

#[test]
fn single_vocalist_with_background_keeps_agent() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata">
          <body>
            <div>
              <p begin="00:01.000" ttm:agent="v1"><span>Main line</span></p>
              <p begin="00:01.200" ttm:agent="v1"><span ttm:role="x-bg">bg</span></p>
            </div>
          </body>
        </tt>
    "#;
    let lrc = lrc(ttml);
    assert!(lrc.contains("{agent:v1}"), "{lrc}");
    assert!(lrc.contains("{bg}bg"), "{lrc}");
}

#[test]
fn single_vocalist_not_v1_is_preserved() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata">
          <body>
            <div>
              <p begin="00:01.000" ttm:agent="v2"><span>Only singer is v2</span></p>
            </div>
          </body>
        </tt>
    "#;
    assert!(lrc(ttml).contains("{agent:v2}"));
}

#[test]
fn lyric_offset_shifts_lines_and_words() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml">
          <head>
            <metadata>
              <audio lyricOffset="10.5"/>
            </metadata>
          </head>
          <body>
            <div>
              <p begin="00:01.000">
                <span begin="00:01.000" end="00:02.000">Hello</span>
              </p>
            </div>
          </body>
        </tt>
    "#;
    let lrc = lrc(ttml);
    assert!(lrc.contains("[00:11.50]"), "{lrc}");
    assert!(lrc.contains("Hello:11.5:12.5"), "{lrc}");
}

#[test]
fn translation_and_roman_spans_are_skipped() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata">
          <body>
            <div>
              <p begin="00:01.000">
                <span begin="00:01.000" end="00:02.000">Main lyric</span>
                <span ttm:role="x-roman">Romanization</span>
                <span ttm:role="x-translation">Translation</span>
              </p>
            </div>
          </body>
        </tt>
    "#;
    let parsed = lines(ttml);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].text, "Main lyric");
}

#[test]
fn split_syllable_spans_are_merged() {
    let ttml = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div>
        <p begin="00:01.000"><span begin="00:01.000" end="00:01.500">hel</span><span begin="00:01.500" end="00:02.000">lo</span></p>
    </div></body></tt>"#;
    let parsed = lines(ttml);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].text, "hello");
    assert_eq!(parsed[0].words.len(), 1);
    assert_eq!(parsed[0].words[0].text, "hello");
    assert_eq!(parsed[0].words[0].start_ms, 1000);
    assert_eq!(parsed[0].words[0].end_ms, 2000);
}

#[test]
fn multiple_background_spans_merge_into_one_line() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata">
          <body>
            <div>
              <p begin="00:01.000">
                <span begin="00:01.000" end="00:02.000">Main lyric</span>
                <span ttm:role="x-bg" begin="00:01.200">
                  <span begin="00:01.200" end="00:01.500">Bg1</span>
                </span>
                <span ttm:role="x-bg" begin="00:01.600">
                  <span begin="00:01.600" end="00:01.900">Bg2</span>
                </span>
              </p>
            </div>
          </body>
        </tt>
    "#;
    let lrc = lrc(ttml);
    assert!(lrc.contains("{bg}Bg1 Bg2"), "{lrc}");
    assert!(lrc.contains("<Bg1:1.2:1.5|Bg2:1.6:1.9>"), "{lrc}");
}

#[test]
fn v2000_agent_mapping() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata">
          <head>
            <metadata>
              <ttm:agent xml:id="v1" type="person"/>
              <ttm:agent xml:id="v2000" type="other"/>
            </metadata>
          </head>
          <body>
            <div>
              <p begin="00:01.000" ttm:agent="v1"><span>Main vocal</span></p>
              <p begin="00:02.000" ttm:agent="v2000"><span>Outro vocal</span></p>
            </div>
          </body>
        </tt>
    "#;
    let lrc = lrc(ttml);
    assert!(lrc.contains("{agent:v1}Main vocal"), "{lrc}");
    assert!(lrc.contains("{agent:v2}Outro vocal"), "{lrc}");
}

#[test]
fn sequential_background_de_duplication() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata">
          <head>
            <metadata>
              <ttm:agent xml:id="v1" type="person"/>
              <ttm:agent xml:id="v2" type="person"/>
            </metadata>
          </head>
          <body>
            <div>
              <p begin="00:01.000" ttm:role="x-bg" ttm:agent="v1"><span>Line 1</span></p>
              <p begin="00:02.000" ttm:role="x-bg" ttm:agent="v1"><span>Line 2</span></p>
              <p begin="00:03.000" ttm:agent="v1">
                <span>Main Line</span>
                <span ttm:role="x-bg" begin="00:03.500"><span>Nested Bg</span></span>
              </p>
              <p begin="00:04.000" ttm:role="x-bg" ttm:agent="v1"><span>Line 4</span></p>
              <p begin="00:05.000" ttm:agent="v2"><span>Line 5</span></p>
            </div>
          </body>
        </tt>
    "#;
    let lrc = lrc(ttml);
    let lines: Vec<&str> = lrc.trim().lines().filter(|l| l.starts_with('[')).collect();
    assert!(lines[0].contains("{bg}Line 1"), "{lrc}");
    assert!(!lines[1].contains("{bg}"), "{lrc}");
    assert!(lines[2].contains("{agent:v1}Main Line"), "{lrc}");
    assert!(lines[3].contains("{bg}Nested Bg"), "{lrc}");
    assert!(!lines[4].contains("{bg}"), "{lrc}");
    assert!(lines[5].contains("{agent:v2}Line 5"), "{lrc}");
}

#[test]
fn ttp_timing_on_paragraph() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter">
          <body>
            <div>
              <p ttp:begin="2.5" ttp:end="4.0"><span>Only timing on ttp</span></p>
            </div>
          </body>
        </tt>
    "#;
    let lrc = lrc(ttml);
    assert!(lrc.contains("[00:02.50]"), "{lrc}");
    assert!(lrc.contains("Only timing on ttp"), "{lrc}");
}

#[test]
fn paragraph_inherits_begin_from_first_span() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml">
          <body>
            <div>
              <p>
                <span begin="00:05.000" end="00:05.500">No</span>
                <span begin="00:05.500" end="00:06.000">begin</span>
                <span begin="00:06.000" end="00:06.500">on p</span>
              </p>
            </div>
          </body>
        </tt>
    "#;
    let lrc = lrc(ttml);
    assert!(lrc.trim().lines().next().unwrap().starts_with("[00:05.00]"));
}

/// Malformed input must not panic.
#[test]
fn malformed_ttml_is_an_error() {
    assert!(parse_ttml("<tt><body><div><p begin=").is_err());
    assert!(parse_ttml("").is_err());
    // Well-formed but empty: no lines, no panic.
    let empty = parse_ttml(r#"<tt xmlns="http://www.w3.org/ns/ttml"><body/></tt>"#).unwrap();
    assert!(empty.lines.is_empty());
    // A `<p>` with no timing at all is skipped.
    let untimed =
        parse_ttml(r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div><p><span>x</span></p></div></body></tt>"#)
            .unwrap();
    assert!(untimed.lines.is_empty());
}

/// The LRC that `to_lrc` writes must parse back into the same timings and markers —
/// that round trip is the actual contract between the TTML and LRC parsers.
#[test]
fn ttml_to_lrc_round_trips_through_the_lrc_parser() {
    let ttml = r#"
        <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata">
          <head><metadata><ttm:agent xml:id="v1" type="person"/></metadata></head>
          <body>
            <div>
              <p begin="00:01.000" ttm:agent="v1">
                <span begin="00:01.000" end="00:01.500">Hello</span>
                <span begin="00:01.600" end="00:02.000">world</span>
              </p>
              <p begin="00:02.500" ttm:role="x-bg"><span>ooh</span></p>
              <p begin="00:03.000" ttm:agent="v1"><span>Next line</span></p>
            </div>
          </body>
        </tt>
    "#;
    let rendered = lrc(ttml);
    let parsed = ml_lyrics::parse_lrc(&rendered).expect("rendered LRC parses");

    assert_eq!(parsed.lines.len(), 3, "{rendered}");
    assert_eq!(parsed.lines[0].start_ms, 1_000);
    assert_eq!(parsed.lines[0].text, "Hello world");
    assert_eq!(parsed.lines[0].agent.as_deref(), Some("v1"));
    assert_eq!(parsed.lines[0].words.len(), 2);
    assert_eq!(parsed.lines[0].words[0].start_ms, 1_000);
    assert_eq!(parsed.lines[0].words[0].end_ms, 1_500);
    assert!(parsed.lines[1].background, "{rendered}");
    assert_eq!(parsed.lines[1].text, "ooh");
    assert!(!parsed.lines[2].background);
    assert_eq!(parsed.lines[2].text, "Next line");
}

/// Every word end in a parsed document is at or after its start, and lines are sorted.
#[test]
fn parsed_documents_are_ordered_and_well_formed() {
    let ttml = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div>
        <p begin="00:01.000"><span begin="00:01.000" end="00:02.000">Hello</span><span begin="00:02.100" end="00:02.900">world</span></p>
        <p begin="00:03.000"><span begin="00:03.000" end="00:04.000">again</span></p>
    </div></body></tt>"#;
    let parsed = lines(ttml);
    assert!(parsed.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
    for line in &parsed {
        for word in &line.words {
            assert!(word.end_ms >= word.start_ms, "{word:?}");
        }
    }
}
