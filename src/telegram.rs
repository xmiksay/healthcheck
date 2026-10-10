use std::time::Duration;

use serde::Serialize;

/// A send must never hang a monitor loop, which awaits it before its next check.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct TelegramClient {
    bot_token: String,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct SendMessageRequest {
    chat_id: i64,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<String>,
}

impl TelegramClient {
    pub fn new(bot_token: String) -> Self {
        Self {
            bot_token,
            client: reqwest::Client::new(),
        }
    }

    async fn send_message_to(&self, chat_id: i64, text: &str) -> anyhow::Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);

        let request = SendMessageRequest {
            chat_id,
            text: text.to_string(),
            parse_mode: Some("HTML".to_string()),
        };

        tracing::debug!("Sending Telegram message to chat_id: {}", chat_id);

        let response = self
            .client
            .post(&url)
            .timeout(SEND_TIMEOUT)
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            tracing::debug!("Telegram message sent successfully");
            Ok(())
        } else {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            tracing::error!(
                "Failed to send Telegram message: {} - {}",
                status,
                error_text
            );
            Err(anyhow::anyhow!(
                "Telegram API error: {} - {}",
                status,
                error_text
            ))
        }
    }

    pub async fn send_alert(
        &self,
        chat_id: i64,
        service_name: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        let formatted_message = format!(
            "🚨 <b>Alert: {}</b>\n\n{}",
            escape_html(service_name),
            escape_html(message)
        );
        self.send_message_to(chat_id, &formatted_message).await
    }

    pub async fn send_recovery(
        &self,
        chat_id: i64,
        service_name: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        let formatted_message = format!(
            "✅ <b>Recovery: {}</b>\n\n{}",
            escape_html(service_name),
            escape_html(message)
        );
        self.send_message_to(chat_id, &formatted_message).await
    }
}

/// Telegram's HTML parse mode rejects the whole message on a stray `&`, `<` or `>`,
/// which failure reasons (URLs, error text) routinely contain.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_entities() {
        assert_eq!(
            escape_html("R&D <api> https://h/?a=1&b=2"),
            "R&amp;D &lt;api&gt; https://h/?a=1&amp;b=2"
        );
    }
}
