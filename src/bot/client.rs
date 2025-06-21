use std::{process::Stdio, time::Duration};

use grammers_client::{
    Client, InputMessage,
    grammers_tl_types::{enums::MessageEntity, types::MessageEntityPre},
    types::{Message, User},
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::{
    process::Command,
    time::{Instant, interval_at},
};

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
                if let Ok(pw) = reader::StdinReader::read("Password: ") {
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
                    const CMD_PREFIXES: [&str; 4] = [",", "，", ".", "。"];

                    let text = m.text();

                    for prefix in CMD_PREFIXES {
                        if text.starts_with(prefix) {
                            let cmd = text.trim_start_matches(prefix);
                            self.handle_cmd(cmd, &m).await?;
                        }
                    }
                }
            }
            _ => (),
        };
        Ok(())
    }

    async fn edit_pre_msg(&self, m: &Message, resp: &str, lang: &str) -> anyhow::Result<()> {
        let trimmed = resp.trim();
        let msg =
            InputMessage::text(trimmed).fmt_entities(vec![MessageEntity::Pre(MessageEntityPre {
                offset: 0,
                length: trimmed.chars().count() as i32,
                language: lang.to_string(),
            })]);
        match m.edit(msg).await {
            Err(grammers_client::InvocationError::Rpc(e)) if e.name == "MESSAGE_NOT_MODIFIED" => {
                Ok(())
            }
            Err(e) => Err(e.into()),
            Ok(_) => Ok(()),
        }
    }

    async fn read_buffer_per_tick<F>(
        &self,
        stdout_reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
        stderr_reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
        msg: &mut String,
        f: &mut F,
    ) -> anyhow::Result<()>
    where
        for<'a> F: AsyncFnMut(&'a str) -> anyhow::Result<()> + Send + 'static,
        for<'a> <F as AsyncFnMut<(&'a str,)>>::CallRefFuture<'a>:
            Future<Output = anyhow::Result<()>> + Send + 'a,
    {
        let mut ticker = interval_at(
            Instant::now()
                .checked_add(Duration::from_millis(800))
                .unwrap(),
            Duration::from_secs(1),
        );
        let mut stdout_done = false;
        let mut stderr_done = false;

        loop {
            tokio::select! {
                res = stdout_reader.next_line(), if !stdout_done => match res {
                    Ok(Some(line)) => {
                        msg.push_str(&line);
                        msg.push('\n');
                    }
                    Ok(None) => stdout_done = true,
                    Err(e) => return Err(e.into()),
                },
                res = stderr_reader.next_line(), if !stderr_done => match res {
                    Ok(Some(line)) => {
                        msg.push_str(&line);
                        msg.push('\n');
                    }
                    Ok(None) => stderr_done = true,
                    Err(e) => return Err(e.into()),
                },
                _ = ticker.tick() => {
                    f(msg).await?;

                    if stdout_done && stderr_done {
                        break;
                    }
                }
                else => break,
            }
        }

        Ok(())
    }

    pub async fn handle_cmd(&self, cmd: &str, m: &Message) -> anyhow::Result<()> {
        let input_msg = format!("❯ {cmd}");
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

        let mut resp = input_msg.clone();
        resp.push('\n');

        let mut child = match Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                resp.push_str(&format!("笨！\n{e}"));
                self.edit_pre_msg(m, &resp, "StdErr").await?;
                return Ok(());
            }
        };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let client = self.clone();
        let m2 = m.clone();
        self.read_buffer_per_tick(
            &mut stdout_reader,
            &mut stderr_reader,
            &mut resp,
            &mut async move |resp| client.edit_pre_msg(&m2, resp, "StdOut").await,
        )
        .await?;

        let status = child.wait().await?;
        if status.success() {
            resp.push_str("Done.");
            self.edit_pre_msg(m, &resp, "StdOut").await?;
        } else {
            resp.push_str(&format!("笨！{status}"));
            self.edit_pre_msg(m, &resp, "StdErr").await?;
        }

        Ok(())
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
