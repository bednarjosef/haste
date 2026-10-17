//! Which account a join logs in with.
//!
//! Players join offline with `--username` until the Microsoft login is wired into joining.
//! For testing online mode locally, `HASTE_TEST_ACCOUNT=<file.json>` names a fake account
//! (`{"access_token": .., "uuid": .., "name": ..}`, written by
//! `scripts/checks/fake-mojang-api.py`); it is refused unless both API hosts are overridden
//! (`HASTE_SESSION_HOST`, `HASTE_SERVICES_HOST`), so a test account can never reach Mojang.

use anyhow::Context;
use haste_protocol::{Account, MsaTokens, ProfileKeyPair};

/// The test-only account file variable.
pub const TEST_ACCOUNT_ENV: &str = "HASTE_TEST_ACCOUNT";

/// The account to join with and its chat keys (online accounts only).
pub struct JoinAccount {
    /// Login account.
    pub account: Account,
    /// Profile key pair for signed chat.
    pub keys: Option<ProfileKeyPair>,
}

#[derive(serde::Deserialize)]
struct TestAccount {
    access_token: String,
    uuid: uuid::Uuid,
    name: String,
}

/// The account for this join: the test account when `HASTE_TEST_ACCOUNT` is set (its player
/// certificates fetched from the services host), else offline as `username`.
pub async fn join_account(username: &str) -> anyhow::Result<JoinAccount> {
    let Some(path) = std::env::var_os(TEST_ACCOUNT_ENV) else {
        return Ok(JoinAccount {
            account: Account::Offline {
                username: username.to_owned(),
            },
            keys: None,
        });
    };
    anyhow::ensure!(
        haste_protocol::auth::hosts_overridden(),
        "{TEST_ACCOUNT_ENV} needs {} and {} pointing at a local API",
        haste_protocol::auth::SESSION_HOST_ENV,
        haste_protocol::auth::SERVICES_HOST_ENV
    );
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", std::path::Path::new(&path).display()))?;
    let test: TestAccount = serde_json::from_str(&text).context("parsing the test account")?;
    let endpoints = haste_setup::msa::MsaEndpoints {
        services_base: haste_protocol::auth::services_host(),
        ..haste_setup::msa::MsaEndpoints::from_env()
    };
    let certificates = haste_setup::msa::player_certificates(
        &reqwest::Client::new(),
        &endpoints,
        &test.access_token,
    )
    .await
    .context("fetching player certificates")?;
    let keys = haste_setup::msa::profile_key_pair(&certificates);
    tracing::info!(
        "test account {} ({}), chat keys {}",
        test.name,
        test.uuid,
        if keys.is_some() { "present" } else { "missing" }
    );
    Ok(JoinAccount {
        account: Account::Microsoft(MsaTokens {
            access_token: test.access_token,
            profile_uuid: test.uuid,
            username: test.name,
        }),
        keys,
    })
}
