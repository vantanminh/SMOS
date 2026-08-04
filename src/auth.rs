//! Operator account, sessions, and offline TOTP (RFC 6238) 2FA.

use anyhow::{bail, Context, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use totp_rs::{Algorithm, Secret, TOTP};

const SESSION_TTL_HOURS: i64 = 24;
const PENDING_TTL_MINUTES: i64 = 10;
const MIN_PASSWORD_LEN: usize = 8;
const MAX_PASSWORD_LEN: usize = 128;

/// On-disk account record (password is argon2 PHC string).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountFile {
    pub email: String,
    pub password_hash: String,
    /// Base32 TOTP secret (present when enrolled / pending enable).
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct Session {
    email: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct PendingLogin {
    email: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthStatus {
    pub setup_required: bool,
    pub authenticated: bool,
    pub email: Option<String>,
    pub totp_enabled: bool,
    pub session_ttl_hours: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginResponse {
    pub status: &'static str,
    pub token: Option<String>,
    pub pending_token: Option<String>,
    pub totp_required: bool,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TotpSetupResponse {
    pub secret: String,
    pub otpauth_url: String,
    pub issuer: String,
    pub account: String,
    pub algorithm: &'static str,
    pub digits: u32,
    pub period: u64,
    pub note: &'static str,
}

pub struct AuthManager {
    path: PathBuf,
    account: RwLock<Option<AccountFile>>,
    sessions: RwLock<HashMap<String, Session>>,
    pending: RwLock<HashMap<String, PendingLogin>>,
    /// Temporary secret while enrolling TOTP (not yet enabled).
    totp_enroll: RwLock<Option<String>>,
}

impl AuthManager {
    pub fn load(data_dir: &Path) -> Result<Self> {
        let path = Self::auth_path(data_dir);
        let account = if path.exists() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("read auth file {}", path.display()))?;
            let acc: AccountFile =
                serde_json::from_str(&raw).with_context(|| "parse auth.json")?;
            Some(acc)
        } else {
            None
        };
        Ok(Self {
            path,
            account: RwLock::new(account),
            sessions: RwLock::new(HashMap::new()),
            pending: RwLock::new(HashMap::new()),
            totp_enroll: RwLock::new(None),
        })
    }

    pub fn auth_path(data_dir: &Path) -> PathBuf {
        data_dir.join("auth.json")
    }

    pub fn setup_required(&self) -> bool {
        self.account.read().is_none()
    }

    pub fn account_email(&self) -> Option<String> {
        self.account.read().as_ref().map(|a| a.email.clone())
    }

    pub fn totp_enabled(&self) -> bool {
        self.account
            .read()
            .as_ref()
            .map(|a| a.totp_enabled)
            .unwrap_or(false)
    }

    fn save_account(path: &Path, account: &AccountFile) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(account)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, raw)?;
        fs::rename(&tmp, path)?;
        // Restrict permissions when possible (best-effort on Unix).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    fn validate_email(email: &str) -> Result<String> {
        let email = email.trim().to_lowercase();
        if email.len() < 5 || email.len() > 254 {
            bail!("email must be 5..=254 characters");
        }
        let at = email.find('@').ok_or_else(|| anyhow::anyhow!("invalid email"))?;
        if at == 0 || at == email.len() - 1 {
            bail!("invalid email");
        }
        let domain = &email[at + 1..];
        if !domain.contains('.') {
            bail!("invalid email domain");
        }
        if email.contains(' ') {
            bail!("invalid email");
        }
        Ok(email)
    }

    fn validate_password(password: &str) -> Result<()> {
        if password.len() < MIN_PASSWORD_LEN || password.len() > MAX_PASSWORD_LEN {
            bail!("password must be {MIN_PASSWORD_LEN}..={MAX_PASSWORD_LEN} characters");
        }
        Ok(())
    }

    fn fill_random(buf: &mut [u8]) -> Result<()> {
        getrandom::fill(buf).map_err(|e| anyhow::anyhow!("system RNG failed: {e}"))
    }

    fn hash_password(password: &str) -> Result<String> {
        let mut salt_bytes = [0u8; 16];
        Self::fill_random(&mut salt_bytes)?;
        let salt = SaltString::encode_b64(&salt_bytes)
            .map_err(|e| anyhow::anyhow!("encode salt: {e}"))?;
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| anyhow::anyhow!("hash password: {e}"))?
            .to_string();
        Ok(hash)
    }

    fn verify_password(password: &str, password_hash: &str) -> Result<bool> {
        let parsed = PasswordHash::new(password_hash)
            .map_err(|e| anyhow::anyhow!("invalid stored password hash: {e}"))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }

    fn random_token() -> String {
        let mut bytes = [0u8; 32];
        Self::fill_random(&mut bytes).expect("system RNG");
        data_encoding::HEXLOWER.encode(&bytes)
    }

    fn make_totp(secret_b32: &str, email: &str) -> Result<TOTP> {
        let secret = Secret::Encoded(secret_b32.to_string())
            .to_bytes()
            .map_err(|e| anyhow::anyhow!("invalid totp secret: {e:?}"))?;
        TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret,
            Some("SMOS".into()),
            email.to_string(),
        )
        .map_err(|e| anyhow::anyhow!("totp init: {e:?}"))
    }

    fn generate_totp_secret() -> Result<String> {
        // 20 bytes → standard base32 secret for authenticator apps.
        let mut bytes = [0u8; 20];
        Self::fill_random(&mut bytes)?;
        Ok(data_encoding::BASE32_NOPAD.encode(&bytes))
    }

    pub fn status(&self, session_token: Option<&str>) -> AuthStatus {
        let authenticated = session_token
            .map(|t| self.validate_session(t).is_some())
            .unwrap_or(false);
        let email = if authenticated {
            session_token.and_then(|t| self.validate_session(t))
        } else {
            None
        };
        AuthStatus {
            setup_required: self.setup_required(),
            authenticated,
            email,
            totp_enabled: self.totp_enabled(),
            session_ttl_hours: SESSION_TTL_HOURS,
        }
    }

    pub fn validate_session(&self, token: &str) -> Option<String> {
        if token.is_empty() {
            return None;
        }
        let mut sessions = self.sessions.write();
        let now = Utc::now();
        // Drop expired
        sessions.retain(|_, s| s.expires_at > now);
        sessions.get(token).map(|s| s.email.clone())
    }

    /// First-time onboarding: create the sole operator account.
    pub fn setup_with_totp_option(
        &self,
        email: &str,
        password: &str,
        enable_totp: bool,
    ) -> Result<(LoginResponse, Option<TotpSetupResponse>)> {
        if !self.setup_required() {
            bail!("setup already completed");
        }
        let email = Self::validate_email(email)?;
        Self::validate_password(password)?;
        let password_hash = Self::hash_password(password)?;
        let now = Utc::now();

        let mut totp_payload = None;
        let mut totp_secret = None;
        if enable_totp {
            let secret = Self::generate_totp_secret()?;
            let totp = Self::make_totp(&secret, &email)?;
            totp_payload = Some(TotpSetupResponse {
                secret: secret.clone(),
                otpauth_url: totp.get_url(),
                issuer: "SMOS".into(),
                account: email.clone(),
                algorithm: "SHA1",
                digits: 6,
                period: 30,
                note: "Scan with an authenticator app. OTP codes work offline (no Wi‑Fi) after the secret is stored on your device.",
            });
            totp_secret = Some(secret.clone());
            *self.totp_enroll.write() = Some(secret);
        }

        let account = AccountFile {
            email: email.clone(),
            password_hash,
            totp_secret,
            totp_enabled: false, // enable only after first verified code
            created_at: now,
            updated_at: now,
        };
        Self::save_account(&self.path, &account)?;
        *self.account.write() = Some(account);

        let token = self.create_session(&email);
        Ok((
            LoginResponse {
                status: "ok",
                token: Some(token),
                pending_token: None,
                totp_required: false,
                email: Some(email),
            },
            totp_payload,
        ))
    }

    fn create_session(&self, email: &str) -> String {
        let token = Self::random_token();
        let expires_at = Utc::now() + Duration::hours(SESSION_TTL_HOURS);
        self.sessions.write().insert(
            token.clone(),
            Session {
                email: email.to_string(),
                expires_at,
            },
        );
        token
    }

    pub fn login(&self, email: &str, password: &str) -> Result<LoginResponse> {
        let email = Self::validate_email(email)?;
        let account = self
            .account
            .read()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("setup required"))?;

        if account.email != email {
            // Uniform error to avoid account enumeration.
            bail!("invalid email or password");
        }
        if !Self::verify_password(password, &account.password_hash)? {
            bail!("invalid email or password");
        }

        if account.totp_enabled {
            let pending = Self::random_token();
            let expires_at = Utc::now() + Duration::minutes(PENDING_TTL_MINUTES);
            self.pending.write().insert(
                pending.clone(),
                PendingLogin {
                    email: account.email.clone(),
                    expires_at,
                },
            );
            return Ok(LoginResponse {
                status: "totp_required",
                token: None,
                pending_token: Some(pending),
                totp_required: true,
                email: Some(account.email),
            });
        }

        let token = self.create_session(&account.email);
        Ok(LoginResponse {
            status: "ok",
            token: Some(token),
            pending_token: None,
            totp_required: false,
            email: Some(account.email),
        })
    }

    pub fn verify_login_totp(&self, pending_token: &str, code: &str) -> Result<LoginResponse> {
        let mut pending = self.pending.write();
        let now = Utc::now();
        pending.retain(|_, p| p.expires_at > now);
        let entry = pending
            .remove(pending_token)
            .ok_or_else(|| anyhow::anyhow!("invalid or expired login challenge"))?;

        self.check_totp_code(&entry.email, code)?;
        let token = self.create_session(&entry.email);
        Ok(LoginResponse {
            status: "ok",
            token: Some(token),
            pending_token: None,
            totp_required: false,
            email: Some(entry.email),
        })
    }

    fn check_totp_code(&self, email: &str, code: &str) -> Result<()> {
        let account = self
            .account
            .read()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no account"))?;
        if account.email != email {
            bail!("invalid OTP");
        }
        let secret = account
            .totp_secret
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("TOTP not configured"))?;
        let totp = Self::make_totp(secret, &account.email)?;
        let code = code.trim();
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            bail!("OTP must be 6 digits");
        }
        // Allow ±1 step clock skew (30s windows) — offline TOTP standard.
        if !totp.check_current(code).unwrap_or(false) {
            // try previous/next window manually via timestamp
            let t = Utc::now().timestamp() as u64;
            let ok = totp.check(code, t)
                || totp.check(code, t.saturating_sub(30))
                || totp.check(code, t + 30);
            if !ok {
                bail!("invalid OTP code");
            }
        }
        Ok(())
    }

    pub fn logout(&self, token: &str) {
        self.sessions.write().remove(token);
    }

    /// Begin TOTP enrollment for the logged-in operator (or continue setup enrollment).
    pub fn begin_totp_setup(&self, session_email: &str) -> Result<TotpSetupResponse> {
        let mut account = self
            .account
            .read()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("setup required"))?;
        if account.email != session_email {
            bail!("unauthorized");
        }
        if account.totp_enabled {
            bail!("TOTP already enabled");
        }
        let secret = Self::generate_totp_secret()?;
        let totp = Self::make_totp(&secret, &account.email)?;
        account.totp_secret = Some(secret.clone());
        account.totp_enabled = false;
        account.updated_at = Utc::now();
        Self::save_account(&self.path, &account)?;
        *self.account.write() = Some(account.clone());
        *self.totp_enroll.write() = Some(secret.clone());

        Ok(TotpSetupResponse {
            secret,
            otpauth_url: totp.get_url(),
            issuer: "SMOS".into(),
            account: account.email,
            algorithm: "SHA1",
            digits: 6,
            period: 30,
            note: "Scan with authenticator app or enter the secret manually. Codes work offline — no network required.",
        })
    }

    /// Confirm enrollment with a live OTP code (enables 2FA).
    pub fn enable_totp(&self, session_email: &str, code: &str) -> Result<()> {
        let account = self
            .account
            .read()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("setup required"))?;
        if account.email != session_email {
            bail!("unauthorized");
        }
        if account.totp_secret.is_none() {
            bail!("call totp setup first");
        }
        self.check_totp_code(session_email, code)?;
        let mut account = account;
        account.totp_enabled = true;
        account.updated_at = Utc::now();
        Self::save_account(&self.path, &account)?;
        *self.account.write() = Some(account);
        *self.totp_enroll.write() = None;
        Ok(())
    }

    pub fn disable_totp(&self, session_email: &str, password: &str, code: &str) -> Result<()> {
        let account = self
            .account
            .read()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("setup required"))?;
        if account.email != session_email {
            bail!("unauthorized");
        }
        if !Self::verify_password(password, &account.password_hash)? {
            bail!("invalid password");
        }
        if account.totp_enabled {
            self.check_totp_code(session_email, code)?;
        }
        let mut account = account;
        account.totp_enabled = false;
        account.totp_secret = None;
        account.updated_at = Utc::now();
        Self::save_account(&self.path, &account)?;
        *self.account.write() = Some(account);
        *self.totp_enroll.write() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn setup_login_logout_roundtrip() {
        let dir = tempdir().unwrap();
        let auth = AuthManager::load(dir.path()).unwrap();
        assert!(auth.setup_required());

        let (login, totp) = auth
            .setup_with_totp_option("admin@example.com", "password123", false)
            .unwrap();
        assert!(totp.is_none());
        assert!(!auth.setup_required());
        let token = login.token.unwrap();
        assert_eq!(
            auth.validate_session(&token).as_deref(),
            Some("admin@example.com")
        );

        auth.logout(&token);
        assert!(auth.validate_session(&token).is_none());

        let again = auth.login("admin@example.com", "password123").unwrap();
        assert_eq!(again.status, "ok");
        assert!(again.token.is_some());
    }

    #[test]
    fn rejects_short_password_and_duplicate_setup() {
        let dir = tempdir().unwrap();
        let auth = AuthManager::load(dir.path()).unwrap();
        assert!(auth
            .setup_with_totp_option("a@b.co", "short", false)
            .is_err());
        auth.setup_with_totp_option("a@b.co", "longenough", false)
            .unwrap();
        assert!(auth
            .setup_with_totp_option("other@b.co", "longenough", false)
            .is_err());
    }

    #[test]
    fn totp_enable_and_required_on_login() {
        let dir = tempdir().unwrap();
        let auth = AuthManager::load(dir.path()).unwrap();
        let (login, _) = auth
            .setup_with_totp_option("ops@example.com", "password123", false)
            .unwrap();
        let token = login.token.unwrap();

        let setup = auth.begin_totp_setup("ops@example.com").unwrap();
        assert!(!setup.secret.is_empty());
        assert!(setup.otpauth_url.starts_with("otpauth://"));

        let totp = AuthManager::make_totp(&setup.secret, "ops@example.com").unwrap();
        let code = totp.generate_current().unwrap();
        auth.enable_totp("ops@example.com", &code).unwrap();
        assert!(auth.totp_enabled());

        let step = auth.login("ops@example.com", "password123").unwrap();
        assert!(step.totp_required);
        let pending = step.pending_token.unwrap();
        let code2 = totp.generate_current().unwrap();
        let done = auth.verify_login_totp(&pending, &code2).unwrap();
        assert_eq!(done.status, "ok");
        assert!(done.token.is_some());

        // old session still valid until logout
        assert!(auth.validate_session(&token).is_some());
    }

    #[test]
    fn wrong_password_fails() {
        let dir = tempdir().unwrap();
        let auth = AuthManager::load(dir.path()).unwrap();
        auth.setup_with_totp_option("a@b.co", "password123", false)
            .unwrap();
        assert!(auth.login("a@b.co", "wrong-password").is_err());
    }
}
