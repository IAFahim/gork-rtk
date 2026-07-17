//! Fail-closed Telegram allowlist.

/// Authorized operators for the bot.
#[derive(Debug, Clone)]
pub struct AuthPolicy {
    allowed_user_ids: Vec<i64>,
}

impl AuthPolicy {
    /// Empty allowlist denies everyone.
    pub fn from_csv(csv: &str) -> Self {
        let allowed_user_ids = csv
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|s| {
                let t = s.trim();
                if t.is_empty() {
                    return None;
                }
                t.parse::<i64>().ok()
            })
            .collect();
        Self { allowed_user_ids }
    }

    pub fn is_empty(&self) -> bool {
        self.allowed_user_ids.is_empty()
    }

    /// Fail-closed: must have non-empty allowlist and matching user id.
    pub fn authorize(&self, user_id: Option<i64>) -> bool {
        if self.allowed_user_ids.is_empty() {
            return false;
        }
        match user_id {
            Some(uid) => self.allowed_user_ids.contains(&uid),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_denies() {
        let p = AuthPolicy::from_csv("");
        assert!(!p.authorize(Some(1)));
        assert!(p.is_empty());
    }

    #[test]
    fn allowlist_works() {
        let p = AuthPolicy::from_csv("42, 99");
        assert!(p.authorize(Some(42)));
        assert!(!p.authorize(Some(1)));
        assert!(!p.authorize(None));
    }
}
