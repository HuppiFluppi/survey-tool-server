use strum::EnumString;

#[derive(PartialEq, Debug, Clone, EnumString)]
pub enum ROLES {
    ADMIN,
    USER
}

#[derive(Debug, Clone)]
pub enum AuthSetting {
    None,
    Simple { auth_mapping: std::collections::HashMap<String, Vec<ROLES>> },
}

impl AuthSetting {
    pub fn simple(entries: Vec<(String, String, Vec<ROLES>)>) -> Self {
        Self::Simple { auth_mapping: entries.into_iter().map(|t| (format!("{}:{}", t.0, t.1), t.2)).collect() }
    }
}

#[derive(Debug, Clone)]
pub enum TlsSetting {
    None,
    Pem { cert: String, key: String },
}

impl TlsSetting {
    pub fn off() -> Self {
        TlsSetting::None
    }
    pub fn pem(cert: String, key: String) -> Self {
        TlsSetting::Pem { cert, key }
    }
}
