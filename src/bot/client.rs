use anyhow::Ok;
use grammers_client::{
    Client, InputMessage,
    grammers_tl_types::{
        enums::MessageEntity,
        types::{MessageEntityCode, MessageEntityPre},
    },
    types::{Message, User},
};
use tokio::process::Command;

pub struct TomorinClient {
    pub client: Client,
    pub me: User,
}

use crate::conf::Conf;
use grammers_client::Update::{MessageEdited, NewMessage};
use grammers_client::{Config, SignInError, session::Session};

mod reader {
    use std::io::{self, BufRead as _, Write as _};

    pub struct StdinReader;

    impl StdinReader {
        pub fn read(prompt: &str) -> anyhow::Result<String> {
            let stdout = io::stdout();
            let mut stdout = stdout.lock();
            stdout.write_all(prompt.as_bytes())?;
            stdout.flush()?;

            let stdin = io::stdin();
            let mut stdin = stdin.lock();

            let mut line = String::new();
            stdin.read_line(&mut line)?;
            Ok(line)
        }
    }
}

impl TomorinClient {
    const SESSION: &'static str = "tomorin.session";

    pub async fn new(conf: Conf) -> anyhow::Result<Self> {
        let client = Client::connect(Config {
            session: Session::load_file_or_create(Self::SESSION)?,
            api_id: conf.api_id,
            api_hash: conf.api_hash,
            params: Default::default(),
        })
        .await?;

        if !client.is_authorized().await? {
            let token = client.request_login_code(&conf.phone).await?;
            let code = reader::StdinReader::read("Code: ")?;
            if let Err(SignInError::PasswordRequired(password_token)) =
                client.sign_in(&token, &code).await
            {
                if let Some(pw) = reader::StdinReader::read("Password: ").ok() {
                    client.check_password(password_token, pw.trim()).await?;
                } else {
                    return Err(anyhow::anyhow!("Password is required but not provided"));
                }
            }
        }

        client.session().save_to_file(Self::SESSION)?;

        let me = client.get_me().await?;

        Ok(Self { client, me })
    }

    pub async fn next_update(&self) -> anyhow::Result<grammers_client::Update> {
        self.client.next_update().await.map_err(Into::into)
    }

    pub async fn update(&self, update: grammers_client::Update) -> anyhow::Result<()> {
        match update {
            NewMessage(m) | MessageEdited(m) => {
                if let Some(a) = m.sender()
                    && a.id() == self.me.id()
                {
                    const CMD_PREFIX1: &str = ",";
                    const CMD_PREFIX2: &str = "，";

                    let text = m.text();
                    if text.starts_with(CMD_PREFIX1) || text.starts_with(CMD_PREFIX2) {
                        let cmd = text
                            .trim_start_matches(CMD_PREFIX1)
                            .trim_start_matches(CMD_PREFIX2);
                        self.handle_cmd(cmd, &m).await?;
                    }
                }
            }
            _ => (),
        };
        Ok(())
    }

    pub async fn handle_cmd(&self, cmd: &str, m: &Message) -> anyhow::Result<()> {
        let input_msg = format!("❯ {}", cmd);
        m.edit(
            InputMessage::text(&input_msg).fmt_entities(vec![MessageEntity::Pre(
                MessageEntityPre {
                    offset: 0,
                    length: input_msg.chars().count() as i32,
                    language: "StdOut".to_string(),
                },
            )]),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to edit message: {e}"))?;

        let mut parts = cmd.split_whitespace();
        let program = match parts.next() {
            Some(p) => p,
            None => {
                m.edit("No command given").await?;
                return Ok(());
            }
        };
        let args = parts;

        let output = Command::new(program)
            .args(args)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to spawn command `{}`: {}", program, e))?;

        let mut resp = String::new();
        resp.push_str(&input_msg);
        resp.push_str("\n");
        if !output.stdout.is_empty() {
            resp.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        if !output.stderr.is_empty() {
            resp.push_str(&String::from_utf8_lossy(&output.stderr));
        }

        if resp.is_empty() {
            resp.push_str("Command produced no output");
        };

        let resp = resp.trim_end();

        let mono_msg =
            InputMessage::text(resp).fmt_entities(vec![MessageEntity::Pre(MessageEntityPre {
                offset: 0,
                length: resp.chars().count() as i32,
                language: "StdOut".to_string(),
            })]);

        m.edit(mono_msg)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to edit message: {e}"))
    }
}

impl Clone for TomorinClient {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            me: self.me.clone(),
        }
    }
}
