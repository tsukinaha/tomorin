use std::{env, process::Stdio, time::Duration};

use grammers_client::client::UpdateStream;
use grammers_client::peer::User;
use grammers_client::update::Update;
use grammers_client::{
    Client,
    tl::{enums::MessageEntity, types::MessageEntityPre},
};
use grammers_client::{SenderPool, client::UpdatesConfiguration, message::InputMessage};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::{
    process::Command,
    task::JoinHandle,
    time::{Instant, interval_at},
};

pub struct TomorinClient {
    pub client: Client,
    pub me: User,
    pub start_time: std::time::Instant,
    pub updates: UpdateStream,
    _sender_pool_task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct TomorinHandler {
    client: Client,
    me: User,
    start_time: std::time::Instant,
}

use crate::conf::Conf;
use grammers_client::SignInError;
use grammers_client::update::Message;
use grammers_client::update::Update::{MessageEdited, NewMessage};
use grammers_session::storages::SqliteSession;
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
    const SESSION: &'static str = "tomorin.session.v2";

    pub async fn new(conf: Conf) -> anyhow::Result<Self> {
        let session = SqliteSession::open(Self::SESSION).await?;

        let SenderPool {
            runner,
            updates,
            handle,
        } = SenderPool::new(std::sync::Arc::new(session), conf.api_id).use_ipv6();

        let sender_pool_task = tokio::spawn(async move {
            let _ = runner.run().await;
        });

        let client = Client::new(handle.clone());
        tracing::info!("Client created");
        if !client.is_authorized().await? {
            let token = client
                .request_login_code(&conf.phone, &conf.api_hash)
                .await?;
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
        let updates = client
            .stream_updates(
                updates,
                UpdatesConfiguration {
                    catch_up: true,
                    ..Default::default()
                },
            )
            .await;

        let me = client.get_me().await?;
        let start_time = std::time::Instant::now();

        Ok(Self {
            client,
            me,
            start_time,
            updates,
            _sender_pool_task: sender_pool_task,
        })
    }

    pub fn handler(&self) -> TomorinHandler {
        TomorinHandler {
            client: self.client.clone(),
            me: self.me.clone(),
            start_time: self.start_time,
        }
    }

    pub async fn next_update(&mut self) -> anyhow::Result<Update> {
        self.updates.next().await.map_err(Into::into)
    }
}

impl TomorinHandler {
    pub async fn update(&self, update: Update) -> anyhow::Result<()> {
        match update {
            NewMessage(m) | MessageEdited(m) => {
                if let Some(a) = m.sender()
                    && a.id() == self.me.id()
                {
                    const CMD_PREFIXES: [&str; 4] = [",", "，", ".", "。"];
                    const REPEAT: &str = "+";
                    const EVAL: &str = "r#";
                    const HELP: &str = "h#";
                    const STATUS: &str = "s#";

                    let text = m.text();

                    if text == REPEAT {
                        self.handle_repeat(&m).await?;
                        return Ok(());
                    }

                    if text.starts_with(EVAL) {
                        let code = text.trim_start_matches(EVAL);
                        self.handle_eval(code, &m).await?;
                        return Ok(());
                    }

                    for prefix in CMD_PREFIXES {
                        if text.starts_with(prefix) {
                            let cmd = text.trim_start_matches(prefix);
                            self.handle_cmd(cmd, &m).await?;
                            return Ok(());
                        }
                    }

                    if text.starts_with(HELP) {
                        self.handle_help(&m).await?;
                        return Ok(());
                    }

                    if text.starts_with(STATUS) {
                        self.handle_status(&m).await?;
                        return Ok(());
                    }
                }
            }
            _ => (),
        };
        Ok(())
    }

    async fn handle_eval(&self, code: &str, m: &Message) -> anyhow::Result<()> {
        use crate::eval::EvalClient;
        m.edit("少女祈祷中......").await?;

        let resp = EvalClient::intance().eval(code).await?;
        self.edit_eval_msg(m, code, &resp).await
    }

    async fn edit_eval_msg(&self, m: &Message, code: &str, resp: &str) -> anyhow::Result<()> {
        let code = code.trim();
        let resp = resp.trim();
        let code_entity = MessageEntity::Pre(MessageEntityPre {
            offset: 0,
            length: code.chars().count() as i32,
            language: "Rust".to_string(),
        });

        let resp = format!("\n{resp}");

        let resp_entity = MessageEntity::Pre(MessageEntityPre {
            offset: code_entity.length(),
            length: resp.chars().count() as i32,
            language: "Output".to_string(),
        });

        let text = format!("{code}{resp}");

        let msg = InputMessage::new()
            .text(&text)
            .fmt_entities(vec![code_entity, resp_entity]);

        match m.edit(msg).await {
            Err(grammers_client::InvocationError::Rpc(e)) if e.name == "MESSAGE_NOT_MODIFIED" => {
                Ok(())
            }
            Err(e) => Err(e.into()),
            Ok(_) => Ok(()),
        }
    }

    async fn edit_pre_msg(&self, m: &Message, resp: &str, lang: &str) -> anyhow::Result<()> {
        const MAX_LINES: usize = 30;
        const TRIMMED_HINT: &str = "以上行数被杜叔叔吃掉了！\n";

        let trimmed = resp.trim();
        let line_count = trimmed.lines().count();

        let trimmed = if line_count > MAX_LINES {
            let mut lines = trimmed.lines().rev().take(MAX_LINES).collect::<Vec<&str>>();
            lines.push(TRIMMED_HINT);
            lines.into_iter().rev().collect::<Vec<&str>>().join("\n")
        } else {
            trimmed.to_string()
        };

        let msg = InputMessage::new()
            .text(&trimmed)
            .fmt_entities(vec![MessageEntity::Pre(MessageEntityPre {
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

    async fn read_buffer_per_tick(
        &self,
        stdout_reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
        stderr_reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
        msg: &mut String,
        m: &Message,
        lang: &str,
    ) -> anyhow::Result<()> {
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
                    Ok(Some(line)) => { msg.push_str(&line); msg.push('\n'); }
                    Ok(None) => stdout_done = true,
                    Err(e) => return Err(e.into()),
                },
                res = stderr_reader.next_line(), if !stderr_done => match res {
                    Ok(Some(line)) => { msg.push_str(&line); msg.push('\n'); }
                    Ok(None) => stderr_done = true,
                    Err(e) => return Err(e.into()),
                },
                _ = ticker.tick() => {
                    self.edit_pre_msg(m, msg, lang).await?;
                    if stdout_done && stderr_done { break; }
                }
                else => break,
            }
        }

        Ok(())
    }

    pub async fn handle_cmd(&self, cmd: &str, m: &Message) -> anyhow::Result<()> {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            m.edit("No command given").await?;
            return Ok(());
        }

        let mut resp = format!("❯ {cmd}");
        resp.push('\n');

        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(cmd)
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

        self.read_buffer_per_tick(
            &mut stdout_reader,
            &mut stderr_reader,
            &mut resp,
            m,
            "StdOut",
        )
        .await?;

        Ok(())
    }

    pub async fn handle_help(&self, m: &Message) -> anyhow::Result<()> {
        let help_text = "**Available commands**:
`+` - Reply to forward/repeat the message
`r#<code>` - Evaluate Rust code
`<prefix><command>` - Execute a shell command (e.g., `,ls`, `，ls`, `.ls`, `。ls`)
`s#` - Show bot status
`h#` - Show this help message";
        m.edit(InputMessage::new().markdown(help_text)).await?;
        Ok(())
    }

    pub async fn handle_status(&self, m: &Message) -> anyhow::Result<()> {
        use chrono::Duration;

        let uptime = std::time::Instant::now().duration_since(self.start_time);
        let uptime_fmt = humantime::format_duration(uptime);

        let start_dt = chrono::Local::now() - Duration::from_std(uptime).unwrap();
        let start_time = start_dt.format("%Y-%m-%d %H:%M:%S");

        let current_time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

        let mut sys = sysinfo::System::new_all();
        sys.refresh_all();
        let pid = std::process::id();
        let tomorin_mem_usage = sys
            .process(sysinfo::Pid::from_u32(pid))
            .map(|p| p.memory() as f64 / 1024.0)
            .unwrap_or(0.0);
        let os_version =
            sysinfo::System::long_os_version().unwrap_or_else(|| "Unknown".to_string());

        let version = env!("CARGO_PKG_VERSION");

        let arch = env::consts::ARCH;
        let status_text = format!(
            "**Status**:
OS: {os_version} - {arch}
Start time: {start_time}
Uptime: {uptime_fmt}
Current time: {current_time}
Memory usage: {tomorin_mem_usage} KiB
Tomorin Version: {version}
"
        );
        m.edit(InputMessage::new().markdown(&status_text)).await?;
        Ok(())
    }

    pub async fn handle_repeat(&self, m: &Message) -> anyhow::Result<()> {
        let reply_m = m.get_reply().await?;

        let Some(reply) = reply_m else {
            return Ok(());
        };

        let Some(peer) = reply.peer() else {
            return Ok(());
        };

        let Some(peer) = peer.to_ref().await else {
            return Ok(());
        };

        if reply.forward_to(peer).await.is_err() {
            let mut input_message = InputMessage::new()
                .text(reply.text())
                .fmt_entities(reply.fmt_entities().cloned().unwrap_or_default());

            if let Some(ref media) = reply.media() {
                input_message = input_message.copy_media(media);
            }

            self.client.send_message(peer, input_message).await?;
        }

        m.delete().await?;
        Ok(())
    }
}
