use regex::Regex;
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use url::Url;

use crate::models::{OutboundLink, PageDigest};

pub fn extract_page(url: &str, html: &str) -> PageDigest {
    let document = Html::parse_document(html);
    let base_url = Url::parse(url).ok();
    let title = text_for_first(&document, "title").unwrap_or_else(|| url.to_string());

    let body_chunks = select_text(&document, &["main", "article", "p", "li", "h1", "h2", "h3"]);
    let clean_text = normalize_whitespace(&body_chunks.join(" "));
    let fallback_text = normalize_whitespace(&select_text(&document, &["body"]).join(" "));
    let text = if clean_text.is_empty() {
        fallback_text
    } else {
        clean_text
    };

    let sentences = split_sentences(&text);
    let short_summary = sentences
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let key_points = sentences.iter().take(5).cloned().collect::<Vec<_>>();
    let claims = sentences
        .iter()
        .filter(|sentence| sentence.split_whitespace().count() >= 8)
        .take(6)
        .cloned()
        .collect::<Vec<_>>();
    let entities = extract_entities(&text);
    let outbound_links = extract_links(&document, base_url.as_ref());

    let boilerplate_ratio = estimate_boilerplate_ratio(&text, html);
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let content_hash = format!("{:x}", hasher.finalize());

    PageDigest {
        url: url.to_string(),
        domain: base_url
            .as_ref()
            .and_then(Url::domain)
            .unwrap_or_default()
            .to_string(),
        title,
        clean_text: text,
        short_summary,
        key_points,
        entities,
        claims,
        outbound_links,
        boilerplate_ratio,
        content_hash,
    }
}

fn select_text(document: &Html, selectors: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    for pattern in selectors {
        let Some(selector) = Selector::parse(pattern).ok() else {
            continue;
        };

        for element in document.select(&selector) {
            let text = normalize_whitespace(&element.text().collect::<Vec<_>>().join(" "));
            if !text.is_empty() {
                values.push(text);
            }
        }
    }

    values
}

fn text_for_first(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|element| normalize_whitespace(&element.text().collect::<Vec<_>>().join(" ")))
        .filter(|text| !text.is_empty())
}

fn extract_links(document: &Html, base_url: Option<&Url>) -> Vec<OutboundLink> {
    let Some(selector) = Selector::parse("a[href]").ok() else {
        return Vec::new();
    };

    document
        .select(&selector)
        .filter_map(|element| {
            let href = element.value().attr("href")?;
            let url = match base_url.and_then(|base| base.join(href).ok()) {
                Some(url) if matches!(url.scheme(), "http" | "https") => url.to_string(),
                _ => return None,
            };

            let anchor = normalize_whitespace(&element.text().collect::<Vec<_>>().join(" "));
            Some(OutboundLink { url, anchor })
        })
        .take(64)
        .collect()
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_sentences(text: &str) -> Vec<String> {
    text.split_terminator(['.', '!', '?'])
        .map(str::trim)
        .filter(|sentence| sentence.split_whitespace().count() >= 6)
        .map(ToOwned::to_owned)
        .collect()
}

fn extract_entities(text: &str) -> Vec<String> {
    let regex =
        Regex::new(r"\b(?:[A-Z][a-z]+(?:\s+[A-Z][a-z]+){0,3})\b").expect("valid entity regex");
    let mut entities = Vec::new();
    for capture in regex.find_iter(text).take(20) {
        let value = capture.as_str().trim().to_string();
        if value.len() > 2 && !entities.contains(&value) {
            entities.push(value);
        }
    }
    entities
}

fn estimate_boilerplate_ratio(text: &str, html: &str) -> f32 {
    if html.is_empty() {
        return 1.0;
    }

    let text_len = text.len() as f32;
    let html_len = html.len() as f32;
    let ratio = 1.0 - (text_len / html_len);
    ratio.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::extract_page;

    #[test]
    fn extracts_basic_page_shape() {
        let html = r#"
        <html>
          <head><title>Test Page</title></head>
          <body>
            <main>
              <p>Novel findings appear in this sentence with enough detail to count.</p>
              <p>Another useful sentence describes a second distinct fact for the digest.</p>
            </main>
            <a href="/next">Read more</a>
          </body>
        </html>
        "#;

        let digest = extract_page("https://example.com/start", html);
        assert_eq!(digest.title, "Test Page");
        assert_eq!(digest.outbound_links.len(), 1);
        assert!(!digest.short_summary.is_empty());
    }
}
