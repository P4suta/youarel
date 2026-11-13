use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::Engine as _;
use ring::hmac;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;
use std::time::Duration;
use ureq::http::StatusCode;

const OAUTH_URL: &str = "https://login.ssl.com/oauth2/token";
const AUTHORIZE_URL: &str = "https://cs.ssl.com/csc/v0/credentials/authorize";
const CLIENT_ID: &str = "kaXTRACNijSWsFdRKg_KAfD3fqrBlzMbWs6TwWHwAn8";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
const REPLY_LIMIT: u64 = 64 * 1024;
const CREDENTIAL_REDIRECTS: u32 = 0;
const OTP_PERIOD_SECONDS: u64 = 30;
const MIN_OTP_LIFETIME_SECONDS: u64 = 5;
const OTP_DIGITS: usize = 6;
const OTP_MODULUS: u32 = 1_000_000;
const OTP_OFFSET_MASK: u8 = 0x0f;
const OTP_VALUE_MASK: u32 = 0x7fff_ffff;
const PROBES: [&str; super::WINDOWS_PROGRAM_COUNT] = [
    "youarel authorization diagnostic one",
    "youarel authorization diagnostic two",
];

mod raw {
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "the authorization clock forwards Unix time to the TOTP calculation"
    )]
    pub(super) fn seconds() -> Result<u64, std::time::SystemTimeError> {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "authorization waits for a fresh OTP interval through this effect boundary"
    )]
    pub(super) fn wait(duration: std::time::Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Problem {
    Authentication,
    Otp,
    RateLimit,
    Quota,
    ScanRequired,
    Malware,
    Timestamp,
    FileAccess,
    Arguments,
    Unknown,
}

impl Problem {
    pub(super) fn classify(text: &str) -> Self {
        let text = text.to_ascii_lowercase();
        if text.contains("invalid otp") || text.contains("invalid_otp") {
            Self::Otp
        } else if text.contains("invalid_grant") || text.contains("invalid credentials") {
            Self::Authentication
        } else if text.contains("rate limit") || text.contains("too_many_requests") {
            Self::RateLimit
        } else if text.contains("quota") {
            Self::Quota
        } else if text.contains("hash") && text.contains("scan") {
            Self::ScanRequired
        } else if text.contains("malware object") || text.contains("code object is a malware") {
            Self::Malware
        } else if text.contains("timestamp") || text.contains("time stamp") {
            Self::Timestamp
        } else if text.contains("accessdeniedexception") || text.contains("filenotfoundexception") {
            Self::FileAccess
        } else if text.contains("unknown option:") || text.contains("missing required") {
            Self::Arguments
        } else {
            Self::Unknown
        }
    }

    pub(super) const fn message(self) -> &'static str {
        match self {
            Self::Authentication => "account authentication was rejected",
            Self::Otp => "the one-time password was rejected",
            Self::RateLimit => "the provider rate limit was reached",
            Self::Quota => "the provider quota was exceeded",
            Self::ScanRequired => "the provider requires a matching code scan",
            Self::Malware => "the provider rejected a malware object",
            Self::Timestamp => "timestamp processing failed",
            Self::FileAccess => "the SDK could not access a required file",
            Self::Arguments => "the SDK rejected its arguments",
            Self::Unknown => "the provider returned an unrecognized failure",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    OAuth,
    Authorize,
}

impl Stage {
    const fn endpoint(self) -> &'static str {
        match self {
            Self::OAuth => OAUTH_URL,
            Self::Authorize => AUTHORIZE_URL,
        }
    }
}

struct Inputs<'a> {
    username: &'a str,
    password: &'a str,
    credential: &'a str,
    seed: &'a str,
}

fn otp(seed: &str, seconds: u64) -> Result<String> {
    let key = base64::engine::general_purpose::STANDARD
        .decode(seed)
        .map_err(|_private_error| anyhow!("the signing TOTP seed is not valid base64"))?;
    ensure!(!key.is_empty(), "the signing TOTP seed is empty");
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &key);
    let counter = seconds / OTP_PERIOD_SECONDS;
    let digest = hmac::sign(&key, &counter.to_be_bytes());
    let bytes = digest.as_ref();
    let offset = usize::from(bytes.last().context("TOTP digest is empty")? & OTP_OFFSET_MASK);
    let end = offset
        .checked_add(size_of::<u32>())
        .context("TOTP offset overflows")?;
    let value = bytes
        .get(offset..end)
        .context("TOTP truncation is outside its digest")?;
    let value = u32::from_be_bytes(value.try_into()?) & OTP_VALUE_MASK;
    let value = value % OTP_MODULUS;
    Ok(format!("{value:0OTP_DIGITS$}"))
}

fn required_reply<'a>(reply: &'a Value, field: &str) -> Result<&'a str> {
    ensure!(
        reply.get("error").is_none_or(Value::is_null),
        "the authorization provider returned a conflicting error"
    );
    reply
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("the authorization provider returned no valid {field}"))
}

fn verify_with(
    inputs: &Inputs<'_>,
    seconds: impl FnOnce() -> Result<u64>,
    mut send: impl FnMut(Stage, Option<&str>, &Value) -> Result<Value>,
) -> Result<()> {
    let login = send(
        Stage::OAuth,
        None,
        &json!({
            "username": inputs.username,
            "password": inputs.password,
            "client_id": CLIENT_ID,
            "grant_type": "password",
        }),
    )?;
    let token = required_reply(&login, "access_token")?;
    let hashes: Vec<String> = PROBES
        .iter()
        .map(|probe| {
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(probe.as_bytes()))
        })
        .collect();
    let reply = send(
        Stage::Authorize,
        Some(token),
        &json!({
            "credentialID": inputs.credential,
            "numSignatures": super::WINDOWS_PROGRAM_COUNT,
            "hash": hashes,
            "OTP": otp(inputs.seed, seconds()?)?,
        }),
    )?;
    required_reply(&reply, "SAD")?;
    Ok(())
}

fn parse_reply(status: StatusCode, bytes: &[u8]) -> Result<Value> {
    if status == StatusCode::TOO_MANY_REQUESTS {
        bail!(
            "Windows signing authorization failed: {}",
            Problem::RateLimit.message()
        );
    }
    let reply: Value = serde_json::from_slice(bytes)
        .map_err(|_private_error| anyhow!("the authorization provider returned invalid JSON"))?;
    if status != StatusCode::OK {
        let problem = ["error", "error_description"]
            .into_iter()
            .filter_map(|field| reply.get(field).and_then(Value::as_str))
            .map(Problem::classify)
            .find(|problem| *problem != Problem::Unknown)
            .unwrap_or(Problem::Unknown);
        bail!(
            "Windows signing authorization failed ({status}): {}",
            problem.message()
        );
    }
    Ok(reply)
}

fn current_seconds() -> Result<u64> {
    let seconds = raw::seconds()?;
    let remaining = OTP_PERIOD_SECONDS
        .checked_sub(seconds % OTP_PERIOD_SECONDS)
        .context("the OTP interval is invalid")?;
    if remaining < MIN_OTP_LIFETIME_SECONDS {
        raw::wait(Duration::from_secs(remaining));
    }
    Ok(raw::seconds()?)
}

pub(super) fn verify(username: &str, password: &str, credential: &str, seed: &str) -> Result<()> {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let config = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(REQUEST_TIMEOUT))
        .https_only(true)
        .max_redirects(CREDENTIAL_REDIRECTS)
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let inputs = Inputs {
        username,
        password,
        credential,
        seed,
    };
    verify_with(&inputs, current_seconds, |stage, token, body| {
        let mut request = agent
            .post(stage.endpoint())
            .header("Content-Type", "application/json");
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let request_bytes = serde_json::to_vec(body)?;
        let mut reply = request
            .send(request_bytes.as_slice())
            .map_err(|_private_error| {
                anyhow!("the Windows authorization request failed during transport")
            })?;
        let status = reply.status();
        let response_bytes = reply
            .body_mut()
            .with_config()
            .limit(REPLY_LIMIT)
            .read_to_vec()
            .map_err(|_private_error| {
                anyhow!("the Windows authorization response could not be read within its limit")
            })?;
        parse_reply(status, &response_bytes)
    })?;
    println!("Verified Windows signing authorization without requesting signatures.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Inputs, Problem, Stage, otp, parse_reply, verify_with};
    use anyhow::{Result, bail};
    use serde_json::json;
    use ureq::http::StatusCode;

    const REFERENCE_SEED: &str = "MTIzNDU2Nzg5MDEyMzQ1Njc4OTA=";
    const EXPECTED_OAUTH_URL: &str = "https://login.ssl.com/oauth2/token";
    const EXPECTED_AUTHORIZE_URL: &str = "https://cs.ssl.com/csc/v0/credentials/authorize";

    fn inputs() -> Inputs<'static> {
        Inputs {
            username: "PRIVATE_USERNAME",
            password: "PRIVATE_PASSWORD",
            credential: "PRIVATE_CREDENTIAL",
            seed: REFERENCE_SEED,
        }
    }

    #[test]
    fn totp_matches_rfc6238_sha1_vectors_with_six_digits() {
        for (seconds, expected) in [
            (59, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ] {
            assert_eq!(otp(inputs().seed, seconds).unwrap(), expected);
        }
    }

    #[test]
    fn malformed_totp_seeds_fail_without_disclosing_the_input() {
        for seed in ["", "PRIVATE_SEED?", "${unresolved.reference}"] {
            let error = otp(seed, 59).unwrap_err().to_string();
            assert!(!error.contains(seed) || seed.is_empty());
            assert!(!error.contains("PRIVATE"));
        }
    }

    #[test]
    fn verification_uses_only_login_and_authorization_with_one_token() {
        let inputs = inputs();
        let mut stages = Vec::new();
        verify_with(
            &inputs,
            || Ok(59),
            |stage, token, request| {
                stages.push(stage);
                match stage {
                    Stage::OAuth => {
                        assert_eq!(stage.endpoint(), EXPECTED_OAUTH_URL);
                        assert_eq!(token, None);
                        assert_eq!(request["username"], inputs.username);
                        assert_eq!(request["password"], inputs.password);
                        Ok(json!({"access_token":"PRIVATE_TOKEN"}))
                    }
                    Stage::Authorize => {
                        assert_eq!(stage.endpoint(), EXPECTED_AUTHORIZE_URL);
                        assert_eq!(token, Some("PRIVATE_TOKEN"));
                        assert_eq!(request["credentialID"], inputs.credential);
                        assert_eq!(request["numSignatures"], 2);
                        assert_eq!(request["hash"].as_array().unwrap().len(), 2);
                        assert_eq!(request["OTP"], "287082");
                        assert!(request.get("SAD").is_none());
                        assert!(request.get("signAlgo").is_none());
                        Ok(json!({"SAD":"PRIVATE_ACTIVATION_DATA"}))
                    }
                }
            },
        )
        .unwrap();
        assert_eq!(stages, [Stage::OAuth, Stage::Authorize]);
    }

    #[test]
    fn rejected_authorization_stops_without_a_signature_request() {
        let mut stages = Vec::new();
        let result = verify_with(
            &inputs(),
            || Ok(59),
            |stage, _, _| -> Result<_> {
                stages.push(stage);
                match stage {
                    Stage::OAuth => Ok(json!({"access_token":"PRIVATE_TOKEN"})),
                    Stage::Authorize => bail!("OTP authorization rejected"),
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(stages, [Stage::OAuth, Stage::Authorize]);
    }

    #[test]
    fn malformed_login_and_activation_replies_never_pass() {
        for (login, activation) in [
            (json!({}), json!({"SAD":"PRIVATE_ACTIVATION_DATA"})),
            (
                json!({"access_token":""}),
                json!({"SAD":"PRIVATE_ACTIVATION_DATA"}),
            ),
            (json!({"access_token":"PRIVATE_TOKEN"}), json!({})),
            (json!({"access_token":"PRIVATE_TOKEN"}), json!({"SAD":" "})),
            (
                json!({"access_token":"PRIVATE_TOKEN"}),
                json!({"SAD":false}),
            ),
            (
                json!({"access_token":"PRIVATE_TOKEN","error":"invalid_grant"}),
                json!({"SAD":"PRIVATE_ACTIVATION_DATA"}),
            ),
            (
                json!({"access_token":"PRIVATE_TOKEN"}),
                json!({"SAD":"PRIVATE_ACTIVATION_DATA","error":"invalid_otp"}),
            ),
        ] {
            let result = verify_with(
                &inputs(),
                || Ok(59),
                |stage, _, _| {
                    Ok(match stage {
                        Stage::OAuth => login.clone(),
                        Stage::Authorize => activation.clone(),
                    })
                },
            );
            let error = result.unwrap_err().to_string();
            assert!(!error.contains("PRIVATE"));
        }
    }

    #[test]
    fn provider_errors_disclose_only_fixed_categories() {
        for (description, expected) in [
            ("invalid otp PRIVATE_SEED", Problem::Otp),
            ("invalid_grant PRIVATE_PASSWORD", Problem::Authentication),
            ("rate limit PRIVATE_TOKEN", Problem::RateLimit),
            ("quota exceeded PRIVATE_TOKEN", Problem::Quota),
            (
                "hash needs to be scanned PRIVATE_HASH",
                Problem::ScanRequired,
            ),
            ("malware object PRIVATE_HASH", Problem::Malware),
            (
                "code object is a malware. This code object cannot be signed using eSigner PRIVATE_HASH",
                Problem::Malware,
            ),
            ("timestamp failed PRIVATE_SIGNATURE", Problem::Timestamp),
            ("AccessDeniedException PRIVATE_PATH", Problem::FileAccess),
            ("Unknown option: PRIVATE_ARGUMENT", Problem::Arguments),
            ("PRIVATE_UNKNOWN", Problem::Unknown),
        ] {
            let bytes = serde_json::to_vec(&json!({"error_description":description})).unwrap();
            let error = parse_reply(StatusCode::BAD_REQUEST, &bytes)
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected.message()));
            assert!(!error.contains("PRIVATE"));
        }
    }

    #[test]
    fn structured_errors_take_precedence_without_optional_descriptions() {
        for (reply, expected) in [
            (json!({"error":"invalid_otp"}), Problem::Otp),
            (json!({"error":"invalid_grant"}), Problem::Authentication),
            (json!({"error":"too_many_requests"}), Problem::RateLimit),
            (
                json!({"error":"invalid_otp","error_description":"timestamp failed PRIVATE_TOKEN"}),
                Problem::Otp,
            ),
            (
                json!({"error":"PRIVATE_UNKNOWN","error_description":"invalid otp PRIVATE_SEED"}),
                Problem::Otp,
            ),
        ] {
            let error = parse_reply(
                StatusCode::BAD_REQUEST,
                &serde_json::to_vec(&reply).unwrap(),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(expected.message()));
            assert!(!error.contains("PRIVATE"));
        }
    }

    #[test]
    fn a_rate_limit_response_needs_no_json_payload() {
        let error = parse_reply(StatusCode::TOO_MANY_REQUESTS, b"PRIVATE_GATEWAY_HTML")
            .unwrap_err()
            .to_string();
        assert!(error.contains(Problem::RateLimit.message()));
        assert!(!error.contains("PRIVATE"));
    }

    #[test]
    fn malformed_response_bytes_remain_private() {
        for bytes in [b"PRIVATE_INVALID_JSON".as_slice(), &[u8::MAX]] {
            let error = parse_reply(StatusCode::OK, bytes).unwrap_err().to_string();
            assert!(!error.contains("PRIVATE"));
        }
    }
}
