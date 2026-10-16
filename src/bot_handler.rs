use teloxide::{
    payloads::{AnswerCallbackQuerySetters, EditMessageTextSetters, SendMessageSetters},
    requests::Requester,
    types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message, ParseMode},
    Bot,
};

use crate::{
    supervisor::{Process, SupervisorError, SupervisorService},
    utils::{
        config::{self, ConfigError},
        markdown,
    },
};

/// Errors that can occur while handling a Telegram update.
#[derive(Debug, thiserror::Error)]
pub enum BotHandlerError {
    /// A request to the Telegram API failed.
    #[error("failed to call the Telegram API: {0}")]
    Telegram(#[from] teloxide::RequestError),

    /// A blocking supervisor task panicked or was cancelled.
    #[error("blocking supervisor task failed: {0}")]
    TaskFailed(#[from] tokio::task::JoinError),

    /// Talking to the supervisor XML-RPC server failed.
    #[error(transparent)]
    Supervisor(#[from] SupervisorError),
}

pub struct BotHandler {
    supervisor_service: SupervisorService,
}

impl BotHandler {
    pub fn new() -> Result<Self, ConfigError> {
        Ok(BotHandler {
            supervisor_service: SupervisorService::new(config::env_var("SUPERVISOR_URL")?),
        })
    }

    async fn get_supervisor_process_list(&self) -> Result<Vec<Process>, BotHandlerError> {
        let supervisor_service = self.supervisor_service.clone();
        let process_list =
            tokio::task::spawn_blocking(move || supervisor_service.process_list()).await??;
        Ok(process_list)
    }

    fn create_supervisor_keyboard(&self, process_list: &Vec<Process>) -> InlineKeyboardMarkup {
        let mut keyboard = vec![vec![InlineKeyboardButton::callback(
            "Supervisors 👇".to_owned(),
            "-".to_owned(),
        )]];

        let supervisor_services_button = process_list
            .chunks(2)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|program| {
                        let state_emoji =
                            if program.state == "RUNNING" || program.state == "RESTARTING" {
                                "✅"
                            } else {
                                "❌"
                            };

                        InlineKeyboardButton::callback(
                            format!("{} {}", &program.name, state_emoji),
                            format!("supervisor_{}", &program.name),
                        )
                    })
                    .collect()
            })
            .collect::<Vec<Vec<InlineKeyboardButton>>>();

        keyboard.extend(supervisor_services_button);

        keyboard.extend(vec![
            vec![
                InlineKeyboardButton::callback("Start all programs ✅", "start_supervisors"),
                InlineKeyboardButton::callback("Stop all programs ❌", "stop_supervisors"),
            ],
            vec![InlineKeyboardButton::callback(
                "Reload supervisor 🔄",
                "reload_supervisors",
            )],
        ]);

        InlineKeyboardMarkup::new(keyboard)
    }

    fn format_supervisor_status(&self, process_list: &Vec<Process>) -> String {
        let supervisor_programs = process_list
            .iter()
            .map(|program| {
                let state_emoji = if program.state == "RUNNING" || program.state == "RESTARTING" {
                    "✅"
                } else {
                    "❌"
                };

                format!(
                    "*name*: {}\n*status*: *{}* {}",
                    markdown::replace_special_chars(&program.name),
                    &program.state,
                    state_emoji
                )
            })
            .collect::<Vec<String>>()
            .join(&markdown::replace_special_chars(
                &"\n---------------------------------\n".to_string(),
            ));

        format!(
            "You can see a summary of the supervisor's status:\n\n\n{}\n\n\\.",
            &supervisor_programs
        )
    }

    async fn update_supervisor_message(
        &self,
        bot: &Bot,
        msg: &Message,
        text: String,
        keyboard: InlineKeyboardMarkup,
    ) -> Result<(), BotHandlerError> {
        bot.edit_message_text(msg.chat.id, msg.id, text)
            .parse_mode(ParseMode::MarkdownV2)
            .reply_markup(keyboard)
            .await?;
        Ok(())
    }

    pub async fn start_message_handler(
        &self,
        bot: &Bot,
        msg: &Message,
        is_back: bool,
    ) -> Result<(), BotHandlerError> {
        let process_list = self.get_supervisor_process_list().await?;
        let text = self.format_supervisor_status(&process_list);
        let keyboard = self.create_supervisor_keyboard(&process_list);

        if is_back {
            self.update_supervisor_message(bot, msg, text, keyboard)
                .await?;
        } else {
            bot.send_message(msg.chat.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .reply_markup(keyboard)
                .await?;
        }
        Ok(())
    }

    async fn handle_supervisor_action(
        &self,
        bot: &Bot,
        msg: &Message,
        query: &CallbackQuery,
        supervisor_name: &str,
        action: &str,
    ) -> Result<(), BotHandlerError> {
        let supervisor_name_ref = supervisor_name.to_string();
        let action_ref = action.to_string();
        let supervisor_service = self.supervisor_service.clone();
        let action_result = tokio::task::spawn_blocking(move || match action_ref.as_str() {
            "start" => supervisor_service.start_process(supervisor_name_ref.clone()),
            "stop" => supervisor_service.stop_process(supervisor_name_ref.clone()),
            _ => Ok(()),
        })
        .await?;

        if let Err(error) = action_result {
            log::error!(
                "Error in {} {} supervisor. message: {}",
                action,
                supervisor_name,
                error
            );
            bot.answer_callback_query(&query.id)
                .text(format!(
                    "Error in {} {} supervisor.",
                    action, supervisor_name
                ))
                .show_alert(true)
                .await?;
            return Ok(());
        }

        let process_list = self.get_supervisor_process_list().await?;
        let supervisor_process = process_list
            .iter()
            .find(|program| program.name == supervisor_name);

        if let Some(program) = supervisor_process {
            let state_emoji = if program.state == "RUNNING" || program.state == "RESTARTING" {
                "✅"
            } else {
                "❌"
            };

            let keyboard = vec![
                vec![
                    InlineKeyboardButton::callback(
                        "Start".to_owned(),
                        format!("supervisor_{}_start", &program.name),
                    ),
                    InlineKeyboardButton::callback(
                        "Stop".to_owned(),
                        format!("supervisor_{}_stop", &program.name),
                    ),
                ],
                vec![InlineKeyboardButton::callback(
                    "Back 🔙".to_owned(),
                    "back_to_home",
                )],
            ];

            let text = format!(
                "*name*: {}\n*status*: *{}* {}\nuptime: {}\n\n\\.",
                markdown::replace_special_chars(&program.name),
                &program.state,
                state_emoji,
                markdown::replace_special_chars(&program.uptime)
            );

            self.update_supervisor_message(bot, msg, text, InlineKeyboardMarkup::new(keyboard))
                .await?;

            if action != "manage" {
                bot.answer_callback_query(&query.id)
                    .text(format!(
                        "Supervisor {} {}ed successfully ✅.",
                        supervisor_name, action
                    ))
                    .show_alert(false)
                    .await?;
            }
        } else {
            bot.answer_callback_query(&query.id)
                .text(format!("The supervisor {} not found.", supervisor_name))
                .show_alert(true)
                .await?;
        }

        Ok(())
    }

    pub async fn supervisor_manager_handler(
        &self,
        bot: &Bot,
        msg: &Message,
        query: &CallbackQuery,
        supervisor_name: &str,
    ) -> Result<(), BotHandlerError> {
        self.handle_supervisor_action(bot, msg, query, supervisor_name, "manage")
            .await
    }

    pub async fn supervisor_start_handler(
        &self,
        bot: &Bot,
        msg: &Message,
        query: &CallbackQuery,
        supervisor_name: &str,
    ) -> Result<(), BotHandlerError> {
        self.handle_supervisor_action(bot, msg, query, supervisor_name, "start")
            .await
    }

    pub async fn supervisor_stop_handler(
        &self,
        bot: &Bot,
        msg: &Message,
        query: &CallbackQuery,
        supervisor_name: &str,
    ) -> Result<(), BotHandlerError> {
        self.handle_supervisor_action(bot, msg, query, supervisor_name, "stop")
            .await
    }

    pub async fn supervisor_stop_all_handler(
        &self,
        bot: &Bot,
        msg: &Message,
        query: &CallbackQuery,
    ) -> Result<(), BotHandlerError> {
        let supervisor_service = self.supervisor_service.clone();
        let stop_all_result =
            tokio::task::spawn_blocking(move || supervisor_service.stop_all_process()).await?;

        if let Err(error) = stop_all_result {
            log::error!("Error in stop all supervisor programs. message: {}", error);
            bot.answer_callback_query(&query.id)
                .text("Error in stop all supervisor programs.")
                .show_alert(true)
                .await?;
        } else {
            bot.answer_callback_query(&query.id)
                .text("All supervisor programs stopped successfully ✅.")
                .show_alert(true)
                .await?;

            let process_list = self.get_supervisor_process_list().await?;
            let text = self.format_supervisor_status(&process_list);
            let keyboard = self.create_supervisor_keyboard(&process_list);

            self.update_supervisor_message(bot, msg, text, keyboard)
                .await?;
        }

        Ok(())
    }

    pub async fn supervisor_start_all_handler(
        &self,
        bot: &Bot,
        msg: &Message,
        query: &CallbackQuery,
    ) -> Result<(), BotHandlerError> {
        let supervisor_service = self.supervisor_service.clone();
        let start_all_result =
            tokio::task::spawn_blocking(move || supervisor_service.start_all_process()).await?;

        if let Err(error) = start_all_result {
            log::error!("Error in start all supervisor programs. message: {}", error);
            bot.answer_callback_query(&query.id)
                .text("Error in start all supervisor programs.")
                .show_alert(true)
                .await?;
        } else {
            bot.answer_callback_query(&query.id)
                .text("All supervisor programs started successfully ✅.")
                .show_alert(true)
                .await?;

            let process_list = self.get_supervisor_process_list().await?;
            let text = self.format_supervisor_status(&process_list);
            let keyboard = self.create_supervisor_keyboard(&process_list);

            self.update_supervisor_message(bot, msg, text, keyboard)
                .await?;
        }

        Ok(())
    }

    pub async fn supervisor_reload_handler(
        &self,
        bot: &Bot,
        msg: &Message,
        query: &CallbackQuery,
    ) -> Result<(), BotHandlerError> {
        let supervisor_service = self.supervisor_service.clone();
        let reload_result =
            tokio::task::spawn_blocking(move || supervisor_service.reload_supervisor()).await?;

        if let Err(error) = reload_result {
            log::error!("Error in reload supervisor. message: {}", error);
            bot.answer_callback_query(&query.id)
                .text("Error in reload supervisor programs.")
                .show_alert(true)
                .await?;
        } else {
            bot.answer_callback_query(&query.id)
                .text("Supervisor reloaded successfully ✅.")
                .show_alert(true)
                .await?;

            let process_list = self.get_supervisor_process_list().await?;
            let text = self.format_supervisor_status(&process_list);
            let keyboard = self.create_supervisor_keyboard(&process_list);

            self.update_supervisor_message(bot, msg, text, keyboard)
                .await?;
        }

        Ok(())
    }
}
