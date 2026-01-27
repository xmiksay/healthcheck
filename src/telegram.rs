use serde::Serialize;

#[derive(Debug, Clone)]
pub struct TelegramClient {
    bot_token: String,
    chat_id: i64,
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
    pub fn new(bot_token: String, chat_id: i64) -> Self {
        Self {
            bot_token,
            chat_id,
            client: reqwest::Client::new(),
        }
    }

    pub async fn send_message(&self, text: &str) -> anyhow::Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);

        let request = SendMessageRequest {
            chat_id: self.chat_id,
            text: text.to_string(),
            parse_mode: Some("HTML".to_string()),
        };

        tracing::debug!("Sending Telegram message to chat_id: {}", self.chat_id);

        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            tracing::debug!("Telegram message sent successfully");
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            tracing::error!("Failed to send Telegram message: {} - {}", status, error_text);
            Err(anyhow::anyhow!("Telegram API error: {} - {}", status, error_text))
        }
    }

    pub async fn send_alert(&self, service_name: &str, message: &str) -> anyhow::Result<()> {
        let formatted_message = format!(
            "🚨 <b>Alert: {}</b>\n\n{}",
            service_name,
            message
        );
        self.send_message(&formatted_message).await
    }

    pub async fn send_recovery(&self, service_name: &str, message: &str) -> anyhow::Result<()> {
        let formatted_message = format!(
            "✅ <b>Recovery: {}</b>\n\n{}",
            service_name,
            message
        );
        self.send_message(&formatted_message).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_client() {
        let client = TelegramClient::new("test_token".to_string(), 12345);
        assert_eq!(client.chat_id, 12345);
    }
}
