//! Bluesky firehose testing commands

use futures::StreamExt;
use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    config::PatternConfig,
    data_source::{
        bluesky::{BlueskyFilter, BlueskyFirehoseSource},
        traits::DataSource,
    },
};
use std::time::Duration;
use tokio::time::timeout;

use crate::output::Output;

/// Listen to the Jetstream firehose with filters
pub async fn listen(
    limit: usize,
    nsids: Vec<String>,
    dids: Vec<String>,
    mentions: Vec<String>,
    keywords: Vec<String>,
    languages: Vec<String>,
    endpoint: Option<String>,
    format: String,
    config: &PatternConfig,
) -> Result<()> {
    let output = Output::new();

    // Build filter from CLI args
    let filter = BlueskyFilter {
        nsids: if nsids.is_empty() {
            vec!["app.bsky.feed.post".to_string()] // Default to posts
        } else {
            nsids
        },
        dids,
        mentions,
        keywords,
        languages,
    };

    // Use endpoint from args or config
    let jetstream_endpoint = endpoint.unwrap_or_else(|| {
        config
            .bluesky
            .as_ref()
            .map(|b| b.jetstream_endpoint.clone())
            .unwrap_or_else(|| "wss://jetstream2.us-east.bsky.network/subscribe".to_string())
    });

    output.success("Bluesky Firehose Listener");
    output.info("Endpoint:", &jetstream_endpoint.bright_cyan().to_string());
    output.info("Limit:", &limit.to_string());
    output.section("Filters");

    if !filter.nsids.is_empty() {
        output.list_item(&format!("NSIDs: {}", filter.nsids.join(", ")));
    }
    if !filter.dids.is_empty() {
        output.list_item(&format!("DIDs: {} DIDs", filter.dids.len()));
    }
    if !filter.mentions.is_empty() {
        output.list_item(&format!("Mentions: {}", filter.mentions.join(", ")));
    }
    if !filter.keywords.is_empty() {
        output.list_item(&format!("Keywords: {}", filter.keywords.join(", ")));
    }
    if !filter.languages.is_empty() {
        output.list_item(&format!("Languages: {}", filter.languages.join(", ")));
    }

    println!();
    output.status("Connecting to Jetstream...");

    // Create firehose source
    let mut source = BlueskyFirehoseSource::new("cli-test", jetstream_endpoint)
        .await
        .with_filter(filter);

    // Connect and start streaming
    let mut stream = source.subscribe(None).await.into_diagnostic()?;

    output.success("Connected! Listening for events...");
    println!();

    let mut count = 0;
    while let Some(event_result) = stream.next().await {
        match event_result {
            Ok(event) => {
                count += 1;

                match format.as_str() {
                    "pretty" => {
                        // Pretty print the post
                        output.info(
                            &format!("[{}]", count),
                            &format!(
                                "@{} (at://{})",
                                event.item.handle.bright_cyan(),
                                event.item.did.dimmed()
                            ),
                        );

                        // Show text content
                        output.markdown(&event.item.text);

                        // Show metadata
                        if let Some(reply) = &event.item.reply {
                            output.status(&format!("  → Reply to {}", reply.parent.uri.dimmed()));
                        }

                        if !event.item.facets.is_empty() {
                            let mentions: Vec<_> = event
                                .item
                                .facets
                                .iter()
                                .filter_map(|f| {
                                    f.features.iter().find_map(|feat| match feat {
                                        pattern_core::data_source::bluesky::FacetFeature::Mention { did } => {
                                            Some(format!("@{}", did))
                                        }
                                        _ => None,
                                    })
                                })
                                .collect();

                            if !mentions.is_empty() {
                                output.status(&format!(
                                    "  → Mentions: {}",
                                    mentions.join(", ").bright_blue()
                                ));
                            }
                        }

                        if !event.item.langs.is_empty() {
                            output.status(&format!(
                                "  → Languages: {}",
                                event.item.langs.join(", ").dimmed()
                            ));
                        }

                        output.info("", "");
                    }
                    "json" => {
                        // JSON output
                        let json = serde_json::to_string_pretty(&event.item).into_diagnostic()?;
                        tracing::info!("{}", json);
                    }
                    "raw" => {
                        // Raw debug output
                        tracing::info!("{:?}", event);
                    }
                    _ => {
                        output.error(&format!("Unknown format: {}", format));
                        return Ok(());
                    }
                }

                // Check limit
                if limit > 0 && count >= limit {
                    output.success(&format!("Received {} events, stopping.", count));
                    break;
                }
            }
            Err(e) => {
                output.error(&format!("Stream error: {}", e));
                if count == 0 {
                    output.warning("No events received. Check your filters or connection.");
                }
                break;
            }
        }
    }

    output.section("Summary");
    output.info("Total events:", &count.to_string());

    Ok(())
}

/// Test connection to Jetstream
pub async fn test_connection(endpoint: Option<String>, config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    // Use endpoint from args or config
    let jetstream_endpoint = endpoint.unwrap_or_else(|| {
        config
            .bluesky
            .as_ref()
            .map(|b| b.jetstream_endpoint.clone())
            .unwrap_or_else(|| "wss://jetstream2.us-east.bsky.network/subscribe".to_string())
    });

    output.success("Jetstream Connection Test");
    output.info("Endpoint:", &jetstream_endpoint.bright_cyan().to_string());
    println!();

    output.status("Connecting...");

    // Create a simple filter that should get some events
    let filter = BlueskyFilter {
        nsids: vec!["app.bsky.feed.post".to_string()],
        ..Default::default()
    };

    let mut source = BlueskyFirehoseSource::new("connection-test", jetstream_endpoint)
        .await
        .with_filter(filter);

    // Try to connect and get one event with a timeout
    let mut stream = source.subscribe(None).await?;
    match timeout(Duration::from_secs(10), stream.next()).await {
        Ok(Some(Ok(event))) => {
            output.success("✓ Connection successful!");
            output.info(
                "Received event from:",
                &event.item.handle.bright_green().to_string(),
            );
            output.info("Event URI:", &event.item.uri);
            output.info("Cursor:", &format!("{:?}", event.cursor));
        }
        Ok(Some(Err(e))) => {
            output.error(&format!("Connection failed: {}", e));
            output.warning("The Jetstream endpoint may be down or unreachable.");
        }
        Ok(None) => {
            output.error("Connection closed unexpectedly");
            output.warning("The stream ended without providing any events.");
        }
        Err(_) => {
            output.error("Connection timeout");
            output.warning("Failed to receive any events within 10 seconds.");
            output.info("", "This could mean:");
            output.list_item("The Jetstream endpoint is unreachable");
            output.list_item("Network connectivity issues");
            output.list_item("Firewall blocking WebSocket connections");
        }
    }

    Ok(())
}
