// Most code of this module is copied from https://github.com/upsuper/telegram-rustevalbot

use std::sync::LazyLock;

use anyhow::Ok;

mod eval;
mod types;

use eval::*;
use types::*;

const EVAL_URL: &str = "https://play.rust-lang.org/execute";

#[derive(Clone)]
pub struct EvalClient {
    client: reqwest::Client,
}

impl EvalClient {
    pub fn intance() -> Self {
        static CLIENT: LazyLock<EvalClient> = LazyLock::new(|| EvalClient {
            client: reqwest::Client::new(),
        });

        CLIENT.clone()
    }

    pub async fn eval(&self, code: &str) -> anyhow::Result<String> {
        let code = normalize_unicode_chars(code);
        let code = generate_code_to_send(&code);

        let req = Request {
            channel: Channel::Nightly,
            edition: "2024",
            mode: Mode::Debug,
            crate_type: CrateType::Bin,
            tests: false,
            backtrace: false,
            code,
        };

        let resp = self.client.post(EVAL_URL).json(&req).send().await?;
        let resp = resp.error_for_status()?.json().await?;
        Ok(generate_result_from_response(resp, Channel::Nightly, false))
    }
}

#[tokio::test]
async fn test_eval() {
    let client = EvalClient::intance();
    let code = r#"
        fn main() {
            println!("Hello, world!");
        }
    "#;
    let result = client.eval(code).await.unwrap();
    println!("Eval result: {}", result);
}
