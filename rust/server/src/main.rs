use crate::shared::{AuthSetting, TlsSetting};
use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, ValueEnum};
use std::error::Error;
use std::net::SocketAddr;
use std::str::FromStr;

#[cfg(feature = "grpc")]
mod grpc;

#[cfg(feature = "rest")]
mod rest;

#[cfg(feature = "aws")]
mod aws;

#[cfg(feature = "local")]
mod local;

mod persistence;
mod shared;

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Opt {
    /// gRPC server address
    #[arg(long, env, default_value = "127.0.0.1:1504")]
    #[cfg(feature = "grpc")]
    grpc_address: SocketAddr,

    /// Http/Rest server address
    #[arg(long, env, default_value = "127.0.0.1:80")]
    #[cfg(feature = "rest")]
    rest_address: String,

    #[command(flatten)]
    auth: CliAuthSetting,

    #[command(flatten)]
    tls: CliTlsSetting,

    #[command(flatten)]
    persistence: CliPersistenceSetting,
}

#[derive(Args, Debug)]
struct CliPersistenceSetting {
    /// Persistence type
    #[arg(long, env, value_enum, default_value_t = CliPersistenceType::Local)]
    persistence_type: CliPersistenceType,

    /// The name of the bucket. This stores the survey configuration files. Required if persistence_type is 'aws'
    #[arg(long, env, required_if_eq("persistence_type", "aws"))]
    #[cfg(feature = "aws")]
    persistence_aws_bucket: Option<String>,

    /// The name of the bucket. This stores the survey configuration files. Required if persistence_type is 'aws'
    #[arg(long, env, default_value = "/survey-tool/server/storage/")]
    #[cfg(feature = "aws")]
    persistence_aws_bucket_prefix: String,

    /// The name of the dynamo db table. This stores the survey results and metadata. Required if persistence_type is 'aws'
    #[arg(long, env, required_if_eq("persistence_type", "aws"))]
    #[cfg(feature = "aws")]
    persistence_aws_dynamo_table: Option<String>,

    /// Path where to store the survey configuration files. Used for 'local' persistence
    #[arg(long, env, default_value = "./survey-tool-server-storage/files/")]
    #[cfg(feature = "local")]
    persistence_local_storage_folder: String,

    /// Path where to store the survey results and metadata db. Used for 'local' persistence
    #[arg(long, env, default_value = "./survey-tool-server-storage/db/")]
    #[cfg(feature = "local")]
    persistence_local_db_folder: String,
}

#[derive(Args, Debug)]
struct CliAuthSetting {
    /// Type of auth
    #[arg(long, env, value_enum, default_value_t = CliAuthType::None)]
    auth_setting: CliAuthType,

    /// Auth configuration string. Required when auth_setting is 'simple'.
    /// Format: 'user1:pass1:role1,role2;user2:pass2:role1,role2'
    #[arg(long, env, required_if_eq("auth_setting", "simple"))]
    auth_config: Option<String>,
}

#[derive(Args, Debug)]
struct CliTlsSetting {
    /// Tls setting
    #[arg(long, env, value_enum, default_value_t = CliTlsType::Off)]
    tls_setting: CliTlsType,

    /// File path for cert pem. Required when tls_setting is 'pem'
    #[arg(long, env, required_if_eq("tls_setting", "pem"))]
    tls_cert_pem_file: Option<String>,

    /// File path for key pem. Required when tls_setting is 'pem'
    #[arg(long, env, required_if_eq("tls_setting", "pem"))]
    tls_key_pem_file: Option<String>,
}

#[derive(Debug, Clone, ValueEnum)]
enum CliPersistenceType {
    /// Local setup. Save files and db to filesystem
    #[cfg(feature = "local")]
    Local,
    /// AWS setup. Use S3 and DynamoDB for persistence
    #[cfg(feature = "aws")]
    AWS,
}

#[derive(Debug, Clone, ValueEnum)]
enum CliAuthType {
    /// Disable authentication
    None,
    /// Enable simple authentication
    Simple,
}

#[derive(Debug, Clone, ValueEnum)]
enum CliTlsType {
    /// Turn TLS off
    Off,
    /// Enable TLS via PEM files
    Pem,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // --- init config & setup
    let opt = Opt::parse();

    // auth
    let auth = get_auth_setting(&opt.auth);

    // tls
    let tls = get_tls_setting(&opt.tls).await;

    // setup persistence
    let persistence = persistence::SurveyPersistenceClient::new().await; //setup_persistence(&opt).await?;

    // --- welcome banner
    welcome_banner();

    // --- start operation
    let mut handles = tokio::task::JoinSet::new();

    // setup grpc
    #[cfg(feature = "grpc")]
    {
        println!("Starting gRPC service...");
        handles.spawn(grpc::SurveyApiServer::serve_with_config(opt.grpc_address, persistence, auth, tls));
        println!("  ✅ running");
    }

    // setup rest
    #[cfg(feature = "rest")]
    {
        println!("Starting Rest service...");
        handles.spawn(rest::SurveyApiServer::serve_with_config(opt.rest_address, persistence, auth, tls));
        println!("  ✅ running");
    }

    handles.join_all().await;

    Ok(())
}

fn welcome_banner() {
    let mut features = Vec::new();
    if cfg!(feature = "grpc") {
        features.push("grpc")
    }
    if cfg!(feature = "rest") {
        features.push("rest")
    }
    if cfg!(feature = "local") {
        features.push("local")
    }
    if cfg!(feature = "aws") {
        features.push("aws")
    }
    println!("###--------------------------------------------------------###");
    println!("###               ~~~ Survey Tool Server ~~~               ###");
    println!("###                                                        ###");
    println!("###  Version: {:10}                                   ###", env!("CARGO_PKG_VERSION"));
    println!("###  Enabled features: {:32}    ###", features.join(","));
    println!("###--------------------------------------------------------###");
}

async fn get_tls_setting(tls: &CliTlsSetting) -> TlsSetting {
    match (&tls.tls_setting, &tls.tls_cert_pem_file, &tls.tls_key_pem_file) {
        (CliTlsType::Off, _, _) => shared::TlsSetting::off(),
        (CliTlsType::Pem, Some(cert_file), Some(key_file)) => {
            let cert = match tokio::fs::read_to_string(cert_file).await {
                Ok(s) => s,
                Err(e) => Opt::command().error(ErrorKind::ValueValidation, format!("Error reading cert file: {e}")).exit(),
            };
            let key = match tokio::fs::read_to_string(key_file).await {
                Ok(s) => s,
                Err(e) => Opt::command().error(ErrorKind::ValueValidation, format!("Error reading key file: {e}")).exit(),
            };
            shared::TlsSetting::pem(cert, key)
        },
        _ => Opt::command().error(ErrorKind::MissingRequiredArgument, "Invalid tls config").exit(),
    }
}

fn get_auth_setting(auth: &CliAuthSetting) -> AuthSetting {
    match (&auth.auth_setting, &auth.auth_config) {
        (CliAuthType::None, _) => shared::AuthSetting::None,
        (CliAuthType::Simple, Some(config)) => {
            let entries = config
                .split(';')
                .map(|e| {
                    let elements: Vec<_> = e.split(':').collect();
                    let [u, p, r] = elements.as_slice() else { Opt::command().error(ErrorKind::ValueValidation, "Invalid auth config").exit() };
                    if u.is_empty() || p.is_empty() || r.is_empty() {
                        Opt::command().error(ErrorKind::ValueValidation, "Invalid auth config").exit()
                    }
                    let roles: Vec<_> = r
                        .split(',')
                        .map(|r| {
                            shared::ROLES::from_str(r)
                                .unwrap_or_else(|f| Opt::command().error(ErrorKind::ValueValidation, format!("Invalid roles: {f}")).exit())
                        })
                        .collect();
                    (u.to_string(), p.to_string(), roles)
                })
                .collect();
            shared::AuthSetting::simple(entries)
        },
        _ => Opt::command().error(ErrorKind::MissingRequiredArgument, "Invalid auth config").exit(),
    }
}

// async fn setup_persistence() -> Result<impl SurveyPersistenceClient, Box<dyn Error>> {
//     todo!()
// }
