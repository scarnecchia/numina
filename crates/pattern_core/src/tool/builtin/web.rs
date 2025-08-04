//! Web tool for fetching and searching web content

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    CoreError, Result, context::AgentHandle, data_source::bluesky::PatternHttpClient, tool::AiTool,
};

/// Operation types for web interactions
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum WebOperation {
    Fetch,
    Search,
}

/// Format for web content rendering
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)]
pub enum WebFormat {
    /// Raw HTML
    Html,
    /// Convert to Markdown
    #[default]
    Markdown,
}

/// Input for web interactions
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct WebInput {
    /// The operation to perform
    pub operation: WebOperation,

    /// For fetch: URL to retrieve
    /// For search: search query
    pub query: String,

    /// For fetch: output format (default: markdown)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<WebFormat>,

    /// For search: maximum results (1-20, default: 10)
    #[schemars(default, with = "i64")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Result from a web search
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchResult {
    /// Result title
    pub title: String,
    /// Result URL
    pub url: String,
    /// Result snippet/description
    pub snippet: String,
}

/// Output from web operations
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum WebOutput {
    /// Fetched content
    Content {
        /// The fetched content
        content: String,
        /// Final URL after redirects
        url: String,
        /// Content type from server
        content_type: String,
        /// Format used
        format: WebFormat,
        /// Content length in characters
        content_length: usize,
    },
    /// Search results
    Results {
        /// Search results
        results: Vec<SearchResult>,
        /// Original query
        query: String,
        /// Number of results
        count: usize,
    },
}

/// Web interaction tool
#[derive(Debug, Clone)]
pub struct WebTool<C: surrealdb::Connection + Clone> {
    #[allow(dead_code)]
    pub(crate) handle: AgentHandle<C>,
    client: PatternHttpClient,
}

impl<C: surrealdb::Connection + Clone> WebTool<C> {
    /// Create a new web tool
    pub fn new(handle: AgentHandle<C>) -> Self {
        let client = PatternHttpClient::default();

        Self { handle, client }
    }

    /// Fetch content from a URL
    async fn fetch_url(&self, url: String, format: WebFormat) -> Result<WebOutput> {
        let response = self.client.client.get(&url).send().await.map_err(|e| {
            CoreError::ToolExecutionFailed {
                tool_name: "web".to_string(),
                cause: format!("Failed to fetch URL: {}", e),
                parameters: serde_json::json!({ "url": url }),
            }
        })?;

        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/html")
            .to_string();

        let text = response
            .text()
            .await
            .map_err(|e| CoreError::ToolExecutionFailed {
                tool_name: "web".to_string(),
                cause: format!("Failed to read response body: {}", e),
                parameters: serde_json::json!({ "url": url }),
            })?;

        let content = match format {
            WebFormat::Html => text.clone(),
            WebFormat::Markdown => {
                // Convert HTML to Markdown
                html2md::parse_html(&text)
            }
        };

        Ok(WebOutput::Content {
            content_length: content.chars().count(),
            content,
            url: final_url,
            content_type,
            format,
        })
    }

    /// Search the web using DuckDuckGo
    async fn search_web(&self, query: String, limit: usize) -> Result<WebOutput> {
        let limit = limit.max(1).min(20);

        // Use DuckDuckGo HTML interface
        let search_url = "https://html.duckduckgo.com/html/";
        let response = self
            .client
            .client
            .get(search_url)
            .query(&[("q", &query)])
            .send()
            .await
            .map_err(|e| CoreError::ToolExecutionFailed {
                tool_name: "web".to_string(),
                cause: format!("Search request failed: {}", e),
                parameters: serde_json::json!({ "query": &query }),
            })?;

        let html = response
            .text()
            .await
            .map_err(|e| CoreError::ToolExecutionFailed {
                tool_name: "web".to_string(),
                cause: format!("Failed to read search results: {}", e),
                parameters: serde_json::json!({ "query": &query }),
            })?;

        // Parse search results using scraper
        let document = scraper::Html::parse_document(&html);
        let result_selector = scraper::Selector::parse(".result").unwrap();
        let title_selector = scraper::Selector::parse(".result__title a").unwrap();
        let snippet_selector = scraper::Selector::parse(".result__snippet").unwrap();

        let mut results = Vec::new();

        for (i, result) in document.select(&result_selector).enumerate() {
            if i >= limit {
                break;
            }

            // Extract title and URL
            let title_elem = result.select(&title_selector).next();
            let (title, url) = if let Some(elem) = title_elem {
                let title = elem.text().collect::<String>().trim().to_string();
                let url = elem.value().attr("href").unwrap_or("").to_string();
                (title, url)
            } else {
                continue;
            };

            // Extract snippet
            let snippet = result
                .select(&snippet_selector)
                .next()
                .map(|elem| elem.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            // Skip if no URL
            if url.is_empty() {
                continue;
            }

            results.push(SearchResult {
                title,
                url,
                snippet,
            });
        }

        Ok(WebOutput::Results {
            count: results.len(),
            results,
            query,
        })
    }
}

#[async_trait]
impl<C: surrealdb::Connection + Clone + std::fmt::Debug> AiTool for WebTool<C> {
    type Input = WebInput;
    type Output = WebOutput;

    fn name(&self) -> &str {
        "web"
    }

    fn description(&self) -> &str {
        r#"Interact with the web. Operations: 'fetch' to get content from a URL, 'search' to search the web using DuckDuckGo.

When using 'fetch' you can select format "html" or "md" (default: "md")

The "md" format converts HTML to readable markdown, which is usually better for understanding content.
The "html" format returns raw HTML, useful when you need to see exact formatting or extract specific elements

Important search operators:
- cats dogs: results about cats or dogs
- "cats and dogs": exact term (avoid unless necessary)
- ~"cats and dogs": semantically similar terms
- cats -dogs: reduce results about dogs
- cats +dogs: increase results about dogs
- cats filetype:pdf: search pdfs about cats (supports doc(x), xls(x), ppt(x), html)
- dogs site:example.com: search dogs on example.com
- cats -site:example.com: exclude example.com from results
- intitle:dogs: title contains "dogs"
- inurl:cats: URL contains "cats"

Use this whenever you need current information, facts, news, or anything beyond your training data."#
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        match params.operation {
            WebOperation::Fetch => {
                let format = params.format.unwrap_or_default();
                self.fetch_url(params.query, format).await
            }
            WebOperation::Search => {
                let limit = params.limit.unwrap_or(10).max(1).min(20);
                self.search_web(params.query, limit).await
            }
        }
    }

    fn usage_rule(&self) -> Option<&'static str> {
        Some(
            "Use 'fetch' to retrieve content from specific URLs. \
             Use 'search' to find information on the web. \
             Always check if information is available in memory before searching the web.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_input_serialization() {
        let fetch = WebInput {
            operation: WebOperation::Fetch,
            query: "https://example.com".to_string(),
            format: Some(WebFormat::Markdown),
            limit: None,
        };
        let json = serde_json::to_string(&fetch).unwrap();
        assert!(json.contains("\"operation\":\"fetch\""));
        assert!(json.contains("\"query\":\"https://example.com\""));

        let search = WebInput {
            operation: WebOperation::Search,
            query: "rust programming".to_string(),
            format: None,
            limit: Some(5),
        };
        let json = serde_json::to_string(&search).unwrap();
        assert!(json.contains("\"operation\":\"search\""));
        assert!(json.contains("\"query\":\"rust programming\""));
    }
}
