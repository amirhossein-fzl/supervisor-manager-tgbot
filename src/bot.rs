use regex::{Captures, Regex};
use teloxide::prelude::*;
use tokio::sync::mpsc;

use crate::bot_handler::{BotHandler, BotHandlerError};
use crate::utils::config::{self, ConfigError};

/// Errors that can occur while starting or running the Telegram bot.
#[derive(Debug, thiserror::Error)]
pub enum BotError {
    /// The bot is misconfigured (missing/invalid environment variables).
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// One of the callback routing patterns failed to compile.
    #[error("invalid callback pattern `{pattern}`: {source}")]
    InvalidCallbackPattern {
        pattern: &'static str,
        #[source]
        source: regex::Error,
    },

    /// A Telegram update could not be (de)serialized.
    #[error("failed to (de)serialize a Telegram update: {0}")]
    UpdateJson(#[from] serde_json::Error),

    /// The background dispatcher task panicked or was cancelled.
    #[error("dispatcher task failed: {0}")]
    Dispatcher(#[from] tokio::task::JoinError),

    /// Handling an update failed.
    #[error(transparent)]
    Handler(#[from] BotHandlerError),
}

/// Regexes that map callback query data onto supervisor actions.
///
/// Compiled once at startup so that an invalid pattern is reported before the
/// bot starts polling Telegram, instead of panicking on every update.
#[derive(Debug, Clone)]
struct CallbackPatterns {
    supervisor_start: Regex,
    supervisor_stop: Regex,
    supervisor: Regex,
}

impl CallbackPatterns {
    fn compile() -> Result<Self, BotError> {
        Ok(CallbackPatterns {
            supervisor_start: Self::compile_one("^supervisor_(.*)_start")?,
            supervisor_stop: Self::compile_one("^supervisor_(.*)_stop")?,
            supervisor: Self::compile_one("^supervisor_(.*)")?,
        })
    }

    fn compile_one(pattern: &'static str) -> Result<Regex, BotError> {
        Regex::new(pattern).map_err(|source| BotError::InvalidCallbackPattern { pattern, source })
    }
}

pub struct TelegramBotService {
    bot: Bot,
    handler: BotHandler,
    admin_id: i64,
    patterns: CallbackPatterns,
}

impl TelegramBotService {
    /// Builds the service, validating all configuration up front.
    pub fn new() -> Result<Self, BotError> {
        Ok(TelegramBotService {
            bot: Bot::new(config::env_var("BOT_TOKEN")?),
            handler: BotHandler::new()?,
            admin_id: config::env_var_i64("ADMIN_ID")?,
            patterns: CallbackPatterns::compile()?,
        })
    }

    async fn handle_message(&self, msg: Message) -> Result<(), BotHandlerError> {
        if let Some(text) = msg.text() {
            if text == "/start" && msg.chat.id.0 == self.admin_id {
                self.handler
                    .start_message_handler(&self.bot, &msg, false)
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_callback_query(&self, q: CallbackQuery) -> Result<(), BotHandlerError> {
        let (Some(data), Some(message)) = (&q.data, &q.message) else {
            return Ok(());
        };

        if message.chat.id.0 != self.admin_id {
            return Ok(());
        }

        let result = self.dispatch_callback(data, message, &q).await;

        if let Err(error) = result {
            log::error!("Error in callback query `{}`. message: {}", data, error);
            // Best effort: dismiss the loading state of the button.
            if let Err(answer_error) = self
                .bot
                .answer_callback_query(&q.id)
                .text("Something went wrong. Check the logs for details.")
                .show_alert(true)
                .await
            {
                log::error!(
                    "Failed to answer callback query `{}`. message: {}",
                    data,
                    answer_error
                );
            }
        }

        Ok(())
    }

    /// Routes a callback query onto the matching handler.
    async fn dispatch_callback(
        &self,
        data: &str,
        message: &Message,
        q: &CallbackQuery,
    ) -> Result<(), BotHandlerError> {
        if let Some(captures) = self.patterns.supervisor_start.captures(data) {
            if let Some(name) = captured_name(&captures) {
                self.handler
                    .supervisor_start_handler(&self.bot, message, q, name)
                    .await?;
            }
        } else if let Some(captures) = self.patterns.supervisor_stop.captures(data) {
            if let Some(name) = captured_name(&captures) {
                self.handler
                    .supervisor_stop_handler(&self.bot, message, q, name)
                    .await?;
            }
        } else if let Some(captures) = self.patterns.supervisor.captures(data) {
            if let Some(name) = captured_name(&captures) {
                self.handler
                    .supervisor_manager_handler(&self.bot, message, q, name)
                    .await?;
            }
        } else if data == "start_supervisors" {
            self.handler
                .supervisor_start_all_handler(&self.bot, message, q)
                .await?;
        } else if data == "stop_supervisors" {
            self.handler
                .supervisor_stop_all_handler(&self.bot, message, q)
                .await?;
        } else if data == "reload_supervisors" {
            self.handler
                .supervisor_reload_handler(&self.bot, message, q)
                .await?;
        } else if data == "back_to_home" {
            self.handler
                .start_message_handler(&self.bot, message, true)
                .await?;
        }

        Ok(())
    }

    pub async fn initialize(&self) -> Result<(), BotError> {
        let (tx, mut rx) = mpsc::channel(100);

        let bot_clone = self.bot.clone();
        let tx_clone = tx.clone();

        let message_handler = move |msg: Message| {
            let tx = tx_clone.clone();
            async move {
                match serde_json::to_value(msg) {
                    Ok(value) => {
                        if let Err(error) = tx.send(("message", value)).await {
                            log::error!(
                                "Failed to enqueue incoming message, the bot is shutting down: {}",
                                error
                            );
                        }
                    }
                    Err(error) => {
                        log::error!("Failed to serialize incoming message: {}", error);
                    }
                }
                respond(())
            }
        };

        let callback_handler = move |q: CallbackQuery| {
            let tx = tx.clone();
            async move {
                match serde_json::to_value(q) {
                    Ok(value) => {
                        if let Err(error) = tx.send(("callback", value)).await {
                            log::error!("Failed to enqueue incoming callback query, the bot is shutting down: {}", error);
                        }
                    }
                    Err(error) => {
                        log::error!("Failed to serialize incoming callback query: {}", error);
                    }
                }
                respond(())
            }
        };

        let handler = dptree::entry()
            .branch(Update::filter_message().endpoint(message_handler))
            .branch(Update::filter_callback_query().endpoint(callback_handler));

        let dispatcher = tokio::spawn(async move {
            Dispatcher::builder(bot_clone, handler)
                .enable_ctrlc_handler()
                .build()
                .dispatch()
                .await;
        });

        while let Some((update_type, update_data)) = rx.recv().await {
            let result = match update_type {
                "message" => match serde_json::from_value(update_data) {
                    Ok(msg) => self.handle_message(msg).await.map_err(BotError::from),
                    Err(error) => Err(BotError::UpdateJson(error)),
                },
                "callback" => match serde_json::from_value(update_data) {
                    Ok(q) => self.handle_callback_query(q).await.map_err(BotError::from),
                    Err(error) => Err(BotError::UpdateJson(error)),
                },
                _ => Ok(()),
            };

            if let Err(error) = result {
                log::error!("Error in `{}` update. message: {}", update_type, error);
            }
        }

        // The dispatcher only stops on shutdown; surface a panic if it ever dies.
        dispatcher.await?;

        Ok(())
    }
}

/// Extracts the supervisor name captured by the `^supervisor_(.*)` family of patterns.
fn captured_name<'a>(captures: &'a Captures<'_>) -> Option<&'a str> {
    captures.get(1).map(|m| m.as_str())
}
