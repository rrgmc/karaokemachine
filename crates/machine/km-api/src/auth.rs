//! The admin password and the two access codes, each exchanged for a bearer token that survives a
//! restart.
//!
//! The threat model is a home LAN, not the internet. What this has to stop is a guest with the
//! Wi-Fi password wiping the queue or uninstalling a package — not a determined attacker. So: one
//! admin password and two optional codes, one for [`Access::Queue`] and one for [`Access::Control`].
//! Nobody wants per-user accounts on a karaoke machine. Each is stored only as an `argon2` hash and
//! exchanged for a token that carries its own level and expiry, verified by recomputation.
//!
//! **A machine always has a password.** The first one is generated at first start — see
//! [`generate_factory_pin`] — so there is no state in which an admin route is open.
//!
//! Five things are deliberate:
//!
//! * **There is no token table.** A token is `v2.<level>.<expiry>.<nonce>.<mac>`, where the MAC is
//!   HMAC-SHA256 keyed on the stored admin hash and taken over the level, that level's code hash and
//!   the session epoch. Verification recomputes it, so nothing is held in memory and a restart keeps
//!   everybody logged in.
//! * **Changing or clearing a code ends that level's tokens and no others**, because the code's hash
//!   is inside the signed message.
//! * **The key is the stored hash, so changing the password invalidates every outstanding token**
//!   without any code that says so. The old implementation cleared a map to achieve this; now it
//!   falls out of the construction.
//! * **[`AdminAuth::set_epoch`] is the revocation that would otherwise be lost.** Bumping it kills
//!   every token without touching the password — "sign out everywhere", for a phone left in a taxi.
//!   Revoking *one* session is not possible and the API says so rather than pretending.
//! * **Login is rate-limited per address and for the machine as a whole.** A six-digit PIN is what
//!   this ships with, and per-address limiting alone scales linearly with an attacker's addresses,
//!   which on a LAN are free.
//!
//! **Expiry is wall-clock, and it has to be.** [`std::time::Instant`] is monotonic since boot, so a
//! restart — the exact thing a token now has to survive — resets it. The cost is that a machine
//! whose clock jumps keeps tokens too long or too briefly; the appliance may have no RTC. That is
//! accepted: the bound that matters is the epoch, which no clock can move.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use ctutils::CtEq;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::access::Access;

/// How long a token stays valid.
///
/// Long enough to cover a party without anybody logging in twice, short enough that a phone left on
/// a sofa is not an indefinite key. It matters more than it used to: with no token table there is no
/// way to revoke one session, so this and the epoch are the only two bounds on a leaked token.
pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(12 * 60 * 60);

/// Failed logins allowed from one address inside [`RATE_WINDOW`].
pub const MAX_FAILED_LOGINS: u32 = 5;

/// Failed logins allowed from *everywhere* inside [`RATE_WINDOW`].
///
/// The per-address budget is what stops one impatient guest; this is what stops somebody walking a
/// guess across a subnet's worth of source addresses, which costs them nothing. Set well above what
/// a room full of people fumbling a PIN produces.
pub const MAX_FAILED_LOGINS_GLOBAL: u32 = 20;

/// The window over which failures are counted.
pub const RATE_WINDOW: Duration = Duration::from_secs(60);

/// Digits in a generated factory PIN.
const FACTORY_PIN_DIGITS: usize = 6;

/// The shortest password this will store.
///
/// **Four, and it is a floor rather than a policy.** It was eight until the machine started
/// generating its own six-digit PIN, and a floor above what the product ships would be a rule it
/// breaks itself. There is no complexity rule and there is not
/// going to be one: this guards a karaoke machine on a home LAN, the threat it answers is a guest
/// changing the queue, and a machine that lectured somebody about punctuation would mostly stop them
/// setting a password at all — which is the state that is actually unsafe. What it does prevent is
/// the empty-ish password, which reads as "protected" and is not. Clearing it is `null`, said
/// deliberately.
///
/// **Here rather than beside the route that enforces it, because three surfaces read it.** The JSON
/// route, the owner's page at `/admin/` and `km-admin` all refuse a short password, and each used to
/// say `4` in its own words — one of them counting bytes rather than characters, which made a
/// two-character CJK password legal on one surface and refused on another. It sits beside
/// [`FACTORY_PIN_DIGITS`] because the two are halves of one policy: what the machine gives itself,
/// and the least it will accept instead.
pub const MIN_PASSWORD_CHARS: usize = 4;

/// Bytes of nonce in a token.
const NONCE_BYTES: usize = 8;

/// Bytes of MAC kept in a token. 128 bits is far past what a forger gets to attempt here.
const MAC_BYTES: usize = 16;

/// The one token format this build issues and accepts. It carries a level.
const TOKEN_VERSION: &str = "v2";

/// A successful login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// The bearer token. Opaque to a client; structured, and self-verifying, to us.
    pub token: String,
    /// Seconds until it stops working.
    ///
    /// A duration rather than a timestamp: the machine's clock may be wrong (no RTC battery, no
    /// NTP on a closed LAN), and a remote can subtract from its own clock without trusting ours.
    pub expires_in_secs: u64,
}

/// Why a login failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoginError {
    /// No password has been configured.
    ///
    /// **Kept, but it should now be unreachable in a running machine**: settings generate a factory
    /// PIN at first start. It survives for a caller that builds an [`AdminAuth::disabled`] by hand,
    /// which the tests do, and as the honest answer if a settings file is ever hand-edited to remove
    /// the hash.
    #[error("no password is set on this machine")]
    NotConfigured,
    /// Wrong password.
    #[error("incorrect password")]
    Rejected,
    /// Too many recent failures, from this address or from everywhere.
    #[error("too many attempts; try again in {retry_after_secs}s")]
    RateLimited {
        /// When to try again.
        retry_after_secs: u64,
    },
    /// The stored hash could not be parsed — a corrupt settings file.
    #[error("the stored password hash is unreadable")]
    BadStoredHash,
}

/// Failure bookkeeping for one client address, or for the machine as a whole.
#[derive(Debug, Clone, Copy)]
struct Attempts {
    count: u32,
    window_started: Instant,
}

impl Attempts {
    fn new() -> Self {
        Self {
            count: 0,
            window_started: Instant::now(),
        }
    }

    /// Seconds left of a lockout, if `budget` is spent and the window is still open.
    fn locked_for(&self, budget: u32) -> Option<u64> {
        let elapsed = self.window_started.elapsed();
        if self.count >= budget && elapsed < RATE_WINDOW {
            // Round up, so "try again in 0s" is never reported while still locked out.
            Some((RATE_WINDOW - elapsed).as_secs() + 1)
        } else {
            None
        }
    }

    fn record(&mut self) {
        if self.window_closed() {
            self.count = 0;
            self.window_started = Instant::now();
        }
        self.count += 1;
    }

    /// Whether this entry's window has passed, so it holds nothing worth remembering.
    ///
    /// Both the reset in [`Attempts::record`] and the pruning in `AdminAuth::record_failure` ask
    /// this, so "expired" has one definition rather than two that could drift apart.
    fn window_closed(&self) -> bool {
        self.window_started.elapsed() >= RATE_WINDOW
    }
}

/// The admin password and the parameters every token is verified against.
#[derive(Debug)]
pub struct AdminAuth {
    /// PHC-format `argon2` hash, or `None` on a hand-built disabled instance.
    ///
    /// **Behind a lock because the password can be set while the machine runs.**
    /// `POST /api/v1/admin/password` is what made it change: before that the only way to set one was
    /// a command line that exits, so a startup snapshot was the whole truth.
    ///
    /// It doubles as the HMAC key, which is why changing it logs everybody out for free.
    hash: RwLock<Option<String>>,
    /// Bumped to invalidate every outstanding token without changing the password.
    epoch: RwLock<u64>,
    /// The `argon2` hashes of the queue code and the control code, where the owner set one.
    codes: RwLock<Codes>,
    ttl: Duration,
    attempts: Mutex<HashMap<IpAddr, Attempts>>,
    everywhere: Mutex<Attempts>,
}

impl Default for AdminAuth {
    fn default() -> Self {
        Self::disabled()
    }
}

impl AdminAuth {
    /// No password, so no token can be issued or verified.
    ///
    /// A machine does not reach this state — settings generate a PIN — but a test can, and so can a
    /// hand-edited settings file.
    pub fn disabled() -> Self {
        Self {
            hash: RwLock::new(None),
            epoch: RwLock::new(0),
            codes: RwLock::new(Codes::default()),
            ttl: DEFAULT_TOKEN_TTL,
            attempts: Mutex::new(HashMap::new()),
            everywhere: Mutex::new(Attempts::new()),
        }
    }

    /// Admin mode with an already-hashed password, as loaded from settings.
    pub fn with_hash(hash: impl Into<String>) -> Self {
        Self {
            hash: RwLock::new(Some(hash.into())),
            ..Self::disabled()
        }
    }

    /// Overrides the token lifetime. Chainable.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Sets the session epoch, as loaded from settings. Chainable.
    pub fn with_epoch(self, epoch: u64) -> Self {
        self.set_epoch(epoch);
        self
    }

    /// Sets the two code hashes, as loaded from settings. Chainable.
    pub fn with_codes(self, queue: Option<String>, control: Option<String>) -> Self {
        self.set_code(Access::Queue, queue);
        self.set_code(Access::Control, control);
        self
    }

    /// Sets or clears the code for one level, for this run.
    ///
    /// Every token of that level stops verifying, for [`Self::set_hash`]'s reason: the code's hash
    /// is inside the signed message. A level with no code ignores the call.
    pub fn set_code(&self, level: Access, hash: Option<String>) {
        let mut codes = self
            .codes
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match level {
            Access::Queue => codes.queue = hash,
            Access::Control => codes.control = hash,
            Access::View | Access::Admin => {}
        }
    }

    /// Whether the owner has set a code for this level.
    pub fn code_set(&self, level: Access) -> bool {
        self.code_hash(level).is_some()
    }

    /// The stored hash of one level's code, cloned out from under the lock.
    fn code_hash(&self, level: Access) -> Option<String> {
        let codes = self
            .codes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match level {
            Access::Queue => codes.queue.clone(),
            Access::Control => codes.control.clone(),
            Access::View | Access::Admin => None,
        }
    }

    /// Whether a password is set.
    ///
    /// **Not a permission input.** A route under `/api/v1/admin/` demands a token whatever this
    /// says, and a machine with no hash simply refuses every login. It is here for diagnostics and
    /// for the settings layer, which uses it to decide whether to generate a PIN.
    pub fn is_configured(&self) -> bool {
        self.read_hash().is_some()
    }

    /// Sets or clears the password for this run.
    ///
    /// **Every outstanding token stops verifying, and that is a property of the construction rather
    /// than a step taken here**: the hash is the HMAC key, so a different hash cannot produce the
    /// same MAC. A token issued against the old password does not survive the change, which is the
    /// point of changing one.
    ///
    /// Writing it down is the controller's business; this is the running value. See
    /// `Controller::set_admin_password`.
    pub fn set_hash(&self, hash: Option<String>) {
        *self.write_hash() = hash;
    }

    /// The current session epoch.
    pub fn epoch(&self) -> u64 {
        *self
            .epoch
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Replaces the session epoch, invalidating every outstanding token.
    ///
    /// Password-independent on purpose: "sign everybody out" and "change the password" are different
    /// acts, and an owner who wants the first should not have to perform the second and then tell
    /// everybody the new one.
    pub fn set_epoch(&self, epoch: u64) {
        *self
            .epoch
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = epoch;
    }

    /// The hash lock for writing, recovering from a poisoned one.
    ///
    /// **Every accessor on this type recovers, and on a security type that is a decision worth
    /// stating.** The three shapes that were here before each failed in a different direction, and
    /// none of them said so: `set_hash` and `set_epoch` dropped the write on the floor and returned
    /// as though it had happened, so a password change or a `POST /admin/sessions/reset` reported
    /// success while changing nothing. `epoch()` degraded to `0` — a **fail-open**, because a token
    /// is an HMAC over the hash and the epoch, so tokens minted before the last sign-out-everywhere
    /// would start verifying again. `read_hash()` degraded to `None`, which reads as "this machine
    /// has no password" and would have `/discover` say so to the whole network.
    ///
    /// The likelihood is tiny — these guard a `String` and a `u64`, so poisoning takes a panic
    /// inside a trivial assignment — but the cost of ruling it out is nothing, and one policy that
    /// cannot fail open beats three that fail three ways. A `String` is not made untrustworthy by
    /// an unrelated panic; what a panic under this lock costs is the request it happened in.
    fn write_hash(&self) -> std::sync::RwLockWriteGuard<'_, Option<String>> {
        self.hash
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The stored hash, cloned out from under the lock.
    ///
    /// Cloned rather than borrowed because a guard cannot be returned past this method, and the
    /// alternative -- holding the read lock across an argon2 verify -- would block a rename of the
    /// password behind a login attempt that deliberately takes a long time.
    fn read_hash(&self) -> Option<String> {
        self.hash
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Hashes a password for storage in settings.
    ///
    /// Argon2id at the crate's default parameters. Tuning them is not worth it here: a login on
    /// this machine happens once a session, so the cost of a verify is irrelevant, and the default
    /// is already far past what a GPU makes cheap.
    ///
    /// The salt is the hasher's own, drawn from OS entropy by `argon2`'s `getrandom` feature,
    /// rather than generated here and handed in. The PHC string is the same shape either way.
    pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
        Ok(Argon2::default()
            .hash_password(password.as_bytes())?
            .to_string())
    }

    /// Exchanges the admin password for an admin token.
    ///
    /// The rate limiters are checked before the password, so a flood costs an attacker the wait
    /// rather than a few hundred milliseconds of our CPU per guess. A code is not accepted here:
    /// [`Self::login_any`] is the door for those.
    pub fn login(&self, from: IpAddr, password: &str) -> Result<Grant, LoginError> {
        if self.read_hash().is_none() {
            return Err(LoginError::NotConfigured);
        }
        if let Some(retry_after_secs) = self.throttled_for(from) {
            return Err(LoginError::RateLimited { retry_after_secs });
        }
        if !matches(self.read_hash().as_deref(), password)? {
            self.record_failure(from);
            return Err(LoginError::Rejected);
        }
        self.clear_failures(from);
        self.issue().ok_or(LoginError::NotConfigured)
    }

    /// Exchanges the admin password or either code for a token of the level it grants.
    ///
    /// **The admin password is tried first, then the control code, then the queue code**, so a
    /// caller always gets the highest level the words they typed open. It shares the rate limiter
    /// with [`Self::login`], so a second door is not a second guessing budget.
    pub fn login_any(&self, from: IpAddr, password: &str) -> Result<(Grant, Access), LoginError> {
        if self.read_hash().is_none() {
            return Err(LoginError::NotConfigured);
        }
        if let Some(retry_after_secs) = self.throttled_for(from) {
            return Err(LoginError::RateLimited { retry_after_secs });
        }
        let Some(level) = self.level_of(password)? else {
            self.record_failure(from);
            return Err(LoginError::Rejected);
        };
        self.clear_failures(from);
        let grant = self.issue_for(level).ok_or(LoginError::NotConfigured)?;
        Ok((grant, level))
    }

    /// The level these words open, if they are the admin password or a code.
    ///
    /// **No rate limit, so only the owner's own surfaces call it**: the route that sets a code uses
    /// it to refuse a code that is already the admin password or the other code.
    pub fn level_of(&self, password: &str) -> Result<Option<Access>, LoginError> {
        if matches(self.read_hash().as_deref(), password)? {
            return Ok(Some(Access::Admin));
        }
        for level in [Access::Control, Access::Queue] {
            if matches(self.code_hash(level).as_deref(), password)? {
                return Ok(Some(level));
            }
        }
        Ok(None)
    }

    /// Mints an admin token without checking a password.
    ///
    /// For tests, and for the machine's own in-process surfaces, which are already trusted. `None`
    /// when there is no password to key the MAC with.
    pub fn issue(&self) -> Option<Grant> {
        self.issue_for(Access::Admin)
    }

    /// Mints a token of one level without checking a password.
    ///
    /// `None` when there is no admin password to key the MAC with, for [`Access::View`], which needs
    /// no token, and for a level whose code the owner has not set.
    pub fn issue_for(&self, level: Access) -> Option<Grant> {
        let stored = self.read_hash()?;
        let code = self.code_for_signing(level)?;
        let expires_at = now_secs().saturating_add(self.ttl.as_secs());
        let nonce = random_hex(NONCE_BYTES);
        let mac = sign(&stored, level, &code, self.epoch(), expires_at, &nonce);
        Some(Grant {
            token: format!("{TOKEN_VERSION}.{level}.{expires_at}.{nonce}.{mac}"),
            expires_in_secs: self.ttl.as_secs(),
        })
    }

    /// Whether a token is a currently valid **admin** token.
    pub fn verify(&self, token: &str) -> bool {
        self.access_of(token) == Some(Access::Admin)
    }

    /// The level a token grants, or `None` when it is not currently valid.
    ///
    /// Recomputation, not lookup: the token carries its own level and expiry, and the MAC proves
    /// both were ours. A forged level or expiry changes the MAC input, so raising a token's level or
    /// extending its life requires the key, which is the stored admin hash.
    pub fn access_of(&self, token: &str) -> Option<Access> {
        let stored = self.read_hash()?;
        let (level, expires_at, nonce, presented) = split_token(token)?;
        if expires_at <= now_secs() {
            return None;
        }
        let code = self.code_for_signing(level)?;
        let expected = sign(&stored, level, &code, self.epoch(), expires_at, nonce);
        let matches: bool = expected.as_bytes().ct_eq(presented.as_bytes()).into();
        matches.then_some(level)
    }

    /// What a token of this level signs in place of a code.
    ///
    /// The admin level signs nothing extra, because the key is already its password. A code level
    /// signs its code's hash, and has no tokens at all while that code is not set.
    fn code_for_signing(&self, level: Access) -> Option<String> {
        match level {
            Access::Admin => Some(String::new()),
            Access::Queue | Access::Control => self.code_hash(level),
            Access::View => None,
        }
    }

    /// Seconds the caller must wait, if either budget is spent.
    ///
    /// The per-address lockout is reported when it is longer, so a guest who has burned their own
    /// five is told about theirs rather than about the machine's.
    fn throttled_for(&self, from: IpAddr) -> Option<u64> {
        let mine = self
            .lock_attempts()
            .get(&from)
            .and_then(|a| a.locked_for(MAX_FAILED_LOGINS));
        let global = self
            .everywhere
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .locked_for(MAX_FAILED_LOGINS_GLOBAL);
        match (mine, global) {
            (Some(mine), Some(global)) => Some(mine.max(global)),
            (some, None) | (None, some) => some,
        }
    }

    fn record_failure(&self, from: IpAddr) {
        {
            let mut attempts = self.lock_attempts();
            attempts.entry(from).or_insert_with(Attempts::new).record();
            // **The map is bounded here or it is not bounded at all.** An entry was only ever
            // removed on a *successful* login from that same address, so every address that ever
            // guessed wrong stayed for the life of the process — and an attacker on the LAN has a
            // /64 of IPv6 source addresses to spend, which is both unbounded memory and a way to
            // never meet the per-address budget. Dropping entries whose window has closed costs a
            // scan of a map that, on a home network, holds single digits; the global counter is
            // what actually stops the address-rotating case, and it is untouched by this.
            attempts.retain(|_, a| !a.window_closed());
        }
        self.everywhere
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record();
    }

    /// Forgives this address's failures on a success.
    ///
    /// **The global counter is deliberately not cleared.** One correct login does not vouch for the
    /// hundred wrong ones that came from elsewhere, and clearing it would hand an attacker a reset
    /// button in exchange for one password they already knew.
    fn clear_failures(&self, from: IpAddr) {
        self.lock_attempts().remove(&from);
    }

    /// The per-address failure map, recovering from a poisoned lock.
    ///
    /// See [`AdminAuth::write_hash`] for why every lock on this type recovers. These two were the
    /// rate limiter's own version of the same fault: a poisoned lock made `throttled_for` answer
    /// "not throttled" and `record_failure` forget the failure, so the guessing budget quietly
    /// became unlimited.
    fn lock_attempts(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, Attempts>> {
        self.attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Seconds since the Unix epoch, saturating at 0 if the clock is before it.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// The two code hashes, where the owner set them.
#[derive(Debug, Default)]
struct Codes {
    queue: Option<String>,
    control: Option<String>,
}

/// Whether the words match a stored `argon2` hash. No hash matches nothing.
fn matches(stored: Option<&str>, password: &str) -> Result<bool, LoginError> {
    let Some(stored) = stored else {
        return Ok(false);
    };
    let parsed = PasswordHash::new(stored).map_err(|_| LoginError::BadStoredHash)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// The MAC for one token's parameters, hex-encoded.
///
/// The version string is inside the signed message, not merely a prefix on the token, so a later
/// format cannot be made to verify by relabelling an older one. The level and the code's hash are
/// inside it for the same reason.
fn sign(key: &str, level: Access, code: &str, epoch: u64, expires_at: u64, nonce: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC accepts a key of any length");
    mac.update(
        format!("km-access-{TOKEN_VERSION}|{level}|{code}|{epoch}|{expires_at}|{nonce}").as_bytes(),
    );
    hex(&mac.finalize().into_bytes()[..MAC_BYTES])
}

/// Splits `v2.<level>.<expiry>.<nonce>.<mac>` into its parts.
fn split_token(token: &str) -> Option<(Access, u64, &str, &str)> {
    let mut parts = token.split('.');
    let version = parts.next()?;
    let level = Access::from_word(parts.next()?)?;
    let expires_at = parts.next()?.parse::<u64>().ok()?;
    let nonce = parts.next()?;
    let mac = parts.next()?;
    if version != TOKEN_VERSION || parts.next().is_some() || nonce.is_empty() || mac.is_empty() {
        return None;
    }
    Some((level, expires_at, nonce, mac))
}

/// A PIN for a machine that has never had a password.
///
/// Six digits, and **never a leading zero**: this is read off a television and typed into forms, and
/// `012345` is mangled the moment anything treats a six-digit code as a number.
///
/// # Panics
///
/// If the operating system will not produce entropy. A guessable factory PIN is worse than a machine
/// that refuses to start, and nothing above this can do anything useful with the failure.
pub fn generate_factory_pin() -> String {
    let mut bytes = [0_u8; FACTORY_PIN_DIGITS];
    getrandom::fill(&mut bytes).expect("the OS must be able to produce entropy for a factory PIN");
    let mut pin = String::with_capacity(FACTORY_PIN_DIGITS);
    for (position, byte) in bytes.iter().enumerate() {
        // Modulo bias over ten values out of 256 is a fraction of a percent per digit and buys an
        // attacker nothing worth having against a rate-limited login.
        let digit = if position == 0 {
            1 + byte % 9
        } else {
            byte % 10
        };
        pin.push(char::from(b'0' + digit));
    }
    pin
}

/// Random bytes, hex-encoded.
///
/// # Panics
///
/// If the operating system will not produce entropy — see [`generate_factory_pin`].
fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    getrandom::fill(&mut buffer)
        .expect("the OS must be able to produce entropy for a bearer token");
    hex(&buffer)
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        // Writing to a String cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Pulls a bearer token out of an `Authorization` header value.
///
/// Case-insensitive on the scheme, because clients disagree about capitalising `Bearer`.
pub fn bearer_token(header: &str) -> Option<&str> {
    let (scheme, value) = header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = value.trim();
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCALHOST: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

    fn other() -> IpAddr {
        IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 44))
    }

    fn configured(password: &str) -> AdminAuth {
        AdminAuth::with_hash(AdminAuth::hash_password(password).expect("hashing works"))
    }

    #[test]
    fn a_machine_with_no_password_issues_nothing_and_verifies_nothing() {
        let auth = AdminAuth::disabled();
        assert!(!auth.is_configured());
        assert_eq!(
            auth.login(LOCALHOST, "anything"),
            Err(LoginError::NotConfigured)
        );
        assert!(auth.issue().is_none());
        assert!(!auth.verify("v2.admin.99999999999.aa.bb"));
    }

    #[test]
    fn the_right_password_yields_a_working_token() {
        let auth = configured("hunter2");
        let grant = auth
            .login(LOCALHOST, "hunter2")
            .expect("the password is right");
        assert!(auth.verify(&grant.token));
        assert_eq!(grant.expires_in_secs, DEFAULT_TOKEN_TTL.as_secs());
    }

    #[test]
    fn the_hash_is_not_the_password() {
        let hash = AdminAuth::hash_password("hunter2").expect("hashing works");
        assert!(hash.starts_with("$argon2"));
        assert!(!hash.contains("hunter2"));
    }

    #[test]
    fn two_hashes_of_one_password_differ_because_the_salt_does() {
        let first = AdminAuth::hash_password("hunter2").expect("hashing works");
        let second = AdminAuth::hash_password("hunter2").expect("hashing works");
        assert_ne!(first, second);
        for hash in [first, second] {
            let auth = AdminAuth::with_hash(hash);
            assert!(auth.login(LOCALHOST, "hunter2").is_ok());
        }
    }

    /// The stored password is still argon2, and a settings file written by an older build must still
    /// let its owner in. Replacing the *token* with an HMAC did not replace the *password* hash, and
    /// this is what keeps that honest.
    #[test]
    fn a_hash_written_by_argon2_0_5_still_opens() {
        let written_by_0_5_3 = [
            (
                "hunter2",
                "$argon2id$v=19$m=19456,t=2,p=1$oBxFKOXbFP0lP+HmUpBhrQ\
                 $W7Df36lTwQggg5oRVRGAjIFhAqBPpUEoxjAAz26BYns",
            ),
            (
                "correct horse battery staple",
                "$argon2id$v=19$m=19456,t=2,p=1$/i6v41FgU4p5AlQG1Zl5ZA\
                 $HLvNM6N/LxvPBz+/aDvZ+8SMCngKLLd7uvs0hW4On5w",
            ),
        ];
        for (password, hash) in written_by_0_5_3 {
            let auth = AdminAuth::with_hash(hash);
            let grant = auth.login(LOCALHOST, password);
            assert!(
                grant.is_ok(),
                "argon2 0.5.3 hash for {password:?} should still open"
            );
            assert_eq!(auth.login(LOCALHOST, "not it"), Err(LoginError::Rejected));
        }
    }

    #[test]
    fn the_wrong_password_is_rejected() {
        let auth = configured("hunter2");
        assert_eq!(auth.login(LOCALHOST, "hunter3"), Err(LoginError::Rejected));
    }

    #[test]
    fn a_made_up_token_never_verifies() {
        let auth = configured("hunter2");
        for nonsense in [
            "",
            "0000",
            "v1",
            "v1.a.b.c",
            "v2.99999999999.aa.bb",
            "v2.owner.99999999999.aa.bb",
            "v1.99999999999.aa.bb",
            ".....",
        ] {
            assert!(!auth.verify(nonsense), "{nonsense:?} should not verify");
        }
    }

    /// The whole point of the rewrite: nothing is held in memory, so a second `AdminAuth` built from
    /// the same stored hash — which is what a restart is — accepts a token minted by the first.
    #[test]
    fn a_token_survives_the_process_that_issued_it() {
        let hash = AdminAuth::hash_password("hunter2").expect("hashing works");
        let before = AdminAuth::with_hash(hash.clone());
        let grant = before
            .login(LOCALHOST, "hunter2")
            .expect("the password is right");
        drop(before);

        let after = AdminAuth::with_hash(hash);
        assert!(
            after.verify(&grant.token),
            "a restart must not log everybody out"
        );
    }

    #[test]
    fn changing_the_password_invalidates_every_token() {
        let auth = configured("hunter2");
        let grant = auth
            .login(LOCALHOST, "hunter2")
            .expect("the password is right");
        assert!(auth.verify(&grant.token));

        auth.set_hash(Some(
            AdminAuth::hash_password("something else").expect("hashing works"),
        ));
        assert!(!auth.verify(&grant.token));
    }

    #[test]
    fn bumping_the_epoch_invalidates_every_token_without_touching_the_password() {
        let auth = configured("hunter2");
        let grant = auth
            .login(LOCALHOST, "hunter2")
            .expect("the password is right");
        assert!(auth.verify(&grant.token));

        auth.set_epoch(auth.epoch() + 1);
        assert!(
            !auth.verify(&grant.token),
            "sign-out-everywhere must take effect"
        );
        assert!(
            auth.login(LOCALHOST, "hunter2").is_ok(),
            "the password still works; only the sessions went"
        );
    }

    #[test]
    fn a_token_minted_under_a_later_epoch_does_not_verify_under_an_earlier_one() {
        let auth = configured("hunter2").with_epoch(7);
        let grant = auth.issue().expect("a password is set");
        auth.set_epoch(6);
        assert!(!auth.verify(&grant.token));
    }

    #[test]
    fn an_expired_token_stops_working() {
        let auth = configured("hunter2").with_ttl(Duration::from_secs(0));
        let grant = auth
            .login(LOCALHOST, "hunter2")
            .expect("the password is right");
        assert!(!auth.verify(&grant.token));
    }

    /// Extending a token's life means re-signing it, and the key for that is the stored hash.
    #[test]
    fn editing_the_expiry_out_of_a_token_does_not_extend_it() {
        let auth = configured("hunter2").with_ttl(Duration::from_secs(1));
        let grant = auth.issue().expect("a password is set");
        let (_, _, nonce, mac) = split_token(&grant.token).expect("our own token parses");
        let forged = format!("v2.admin.{}.{nonce}.{mac}", now_secs() + 90_000);
        assert!(!auth.verify(&forged));
    }

    #[test]
    fn repeated_failures_lock_an_address_out() {
        let auth = configured("hunter2");
        for _ in 0..MAX_FAILED_LOGINS {
            assert_eq!(auth.login(LOCALHOST, "wrong"), Err(LoginError::Rejected));
        }
        // The right password waits too: the limiter is checked before the password.
        match auth.login(LOCALHOST, "hunter2") {
            Err(LoginError::RateLimited { retry_after_secs }) => assert!(retry_after_secs > 0),
            other => panic!("expected a lockout, got {other:?}"),
        }
    }

    #[test]
    fn a_lockout_applies_to_one_address_only() {
        let auth = configured("hunter2");
        for _ in 0..MAX_FAILED_LOGINS {
            let _ = auth.login(LOCALHOST, "wrong");
        }
        assert!(auth.login(other(), "hunter2").is_ok());
    }

    #[test]
    fn a_success_forgives_earlier_failures_from_that_address() {
        let auth = configured("hunter2");
        for _ in 0..(MAX_FAILED_LOGINS - 1) {
            let _ = auth.login(LOCALHOST, "wrong");
        }
        assert!(auth.login(LOCALHOST, "hunter2").is_ok());
        for _ in 0..(MAX_FAILED_LOGINS - 1) {
            assert_eq!(auth.login(LOCALHOST, "wrong"), Err(LoginError::Rejected));
        }
    }

    /// Addresses are free on a LAN, so a per-address budget alone is not a limit at all. Each
    /// address here spends less than its own budget and the machine still shuts.
    #[test]
    fn spreading_guesses_across_addresses_still_hits_the_machines_own_limit() {
        let auth = configured("hunter2");
        let mut spent = 0_u32;
        for host in 0..u8::try_from(MAX_FAILED_LOGINS_GLOBAL).expect("fits") {
            let from = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, host));
            if auth.login(from, "wrong") == Err(LoginError::Rejected) {
                spent += 1;
            }
        }
        assert_eq!(
            spent, MAX_FAILED_LOGINS_GLOBAL,
            "each address gets its first guess"
        );
        let fresh = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 1, 1));
        assert!(
            matches!(
                auth.login(fresh, "hunter2"),
                Err(LoginError::RateLimited { .. })
            ),
            "an address that has never guessed is still held by the machine's own budget"
        );
    }

    #[test]
    fn a_corrupt_stored_hash_is_reported_not_ignored() {
        let auth = AdminAuth::with_hash("not a PHC string");
        assert_eq!(
            auth.login(LOCALHOST, "hunter2"),
            Err(LoginError::BadStoredHash)
        );
    }

    /// A token whose MAC is one character off, or cut short, is refused.
    #[test]
    fn a_token_with_a_damaged_mac_never_verifies() {
        let auth = configured("hunter2");
        let token = auth.issue().expect("a password is set").token;
        assert!(auth.verify(&token));

        let (rest, mac) = token.rsplit_once('.').expect("a token has four parts");
        let flipped = if mac.ends_with('0') { '1' } else { '0' };
        let damaged = format!("{rest}.{}{flipped}", &mac[..mac.len() - 1]);
        let shortened = format!("{rest}.{}", &mac[..mac.len() - 1]);
        assert!(!auth.verify(&damaged));
        assert!(!auth.verify(&shortened));
    }

    #[test]
    fn bearer_tokens_are_parsed_out_of_the_header() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer abc"), Some("abc"));
        assert_eq!(bearer_token("BEARER  abc "), Some("abc"));
        assert_eq!(bearer_token("Basic abc"), None);
        assert_eq!(bearer_token("abc"), None);
        assert_eq!(bearer_token("Bearer "), None);
    }

    #[test]
    fn a_factory_pin_is_six_digits_and_never_starts_with_a_zero() {
        for _ in 0..200 {
            let pin = generate_factory_pin();
            assert_eq!(pin.len(), FACTORY_PIN_DIGITS);
            assert!(
                pin.chars().all(|c| c.is_ascii_digit()),
                "{pin} should be digits"
            );
            assert!(!pin.starts_with('0'), "{pin} would be mangled as a number");
        }
    }

    #[test]
    fn factory_pins_do_not_repeat() {
        let first = generate_factory_pin();
        let differs = (0..50).any(|_| generate_factory_pin() != first);
        assert!(
            differs,
            "a constant factory PIN would be the bug this replaced"
        );
    }

    #[test]
    fn a_pin_is_a_password_like_any_other() {
        let pin = generate_factory_pin();
        let auth = configured(&pin);
        assert!(auth.login(LOCALHOST, &pin).is_ok());
        assert_eq!(auth.login(LOCALHOST, "000000"), Err(LoginError::Rejected));
    }

    fn with_codes(queue: &str, control: &str) -> AdminAuth {
        configured("hunter2").with_codes(
            Some(AdminAuth::hash_password(queue).expect("hashing works")),
            Some(AdminAuth::hash_password(control).expect("hashing works")),
        )
    }

    #[test]
    fn each_code_opens_its_own_level_and_the_admin_password_opens_the_top() {
        let auth = with_codes("sing", "boss");
        for (words, level) in [
            ("sing", Access::Queue),
            ("boss", Access::Control),
            ("hunter2", Access::Admin),
        ] {
            let (grant, granted) = auth.login_any(LOCALHOST, words).expect("a known code");
            assert_eq!(granted, level);
            assert_eq!(auth.access_of(&grant.token), Some(level));
            assert_eq!(auth.verify(&grant.token), level == Access::Admin);
        }
        assert_eq!(
            auth.login_any(LOCALHOST, "nope").map(|(_, level)| level),
            Err(LoginError::Rejected)
        );
    }

    #[test]
    fn the_admin_door_takes_no_code() {
        let auth = with_codes("sing", "boss");
        assert_eq!(auth.login(LOCALHOST, "boss"), Err(LoginError::Rejected));
    }

    #[test]
    fn a_level_with_no_code_issues_no_token() {
        let auth = configured("hunter2");
        assert!(auth.issue_for(Access::Queue).is_none());
        assert!(auth.issue_for(Access::Control).is_none());
        assert!(auth.issue_for(Access::View).is_none());
        assert!(auth.issue_for(Access::Admin).is_some());
    }

    #[test]
    fn relabelling_a_token_to_a_higher_level_breaks_its_mac() {
        let auth = with_codes("sing", "boss");
        let grant = auth
            .issue_for(Access::Queue)
            .expect("the queue code is set");
        let raised = grant.token.replacen(".queue.", ".admin.", 1);
        assert_eq!(auth.access_of(&raised), None);
        let raised = grant.token.replacen(".queue.", ".control.", 1);
        assert_eq!(auth.access_of(&raised), None);
    }

    #[test]
    fn changing_a_code_ends_that_levels_tokens_and_no_others() {
        let auth = with_codes("sing", "boss");
        let queue = auth
            .issue_for(Access::Queue)
            .expect("the queue code is set");
        let control = auth
            .issue_for(Access::Control)
            .expect("the control code is set");
        let admin = auth.issue().expect("a password is set");

        auth.set_code(
            Access::Queue,
            Some(AdminAuth::hash_password("sing2").expect("hashing works")),
        );
        assert_eq!(auth.access_of(&queue.token), None);
        assert_eq!(auth.access_of(&control.token), Some(Access::Control));
        assert!(auth.verify(&admin.token));

        auth.set_code(Access::Control, None);
        assert_eq!(auth.access_of(&control.token), None);
        assert!(auth.verify(&admin.token));
    }

    #[test]
    fn level_of_names_what_the_words_open() {
        let auth = with_codes("sing", "boss");
        assert_eq!(auth.level_of("sing"), Ok(Some(Access::Queue)));
        assert_eq!(auth.level_of("hunter2"), Ok(Some(Access::Admin)));
        assert_eq!(auth.level_of("other"), Ok(None));
    }
}
