use std::{env, fs};
use std::time::Duration;
use acme_micro::{Certificate, create_p384_key, Directory, DirectoryUrl};
use actix_cors::Cors;
use actix_files::Files;
use actix_web::{get, web, HttpServer, Responder, App, HttpResponse, rt};
use actix_web::http::header;
use anyhow::anyhow;
use openssl::pkey::PKey;
use openssl::ssl::{SslAcceptor, SslMethod};
use openssl::x509::X509;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use time::format_description::well_known;
use time::{OffsetDateTime, UtcOffset};

#[derive(Debug, Deserialize)]
pub struct TimeReq {
    year: u16,
    week: u8,
    team: String,
    offset: i8,
}

#[derive(Debug, Serialize, Default)]
pub struct TimeResp {
    team: String,
    start: String,
    kickoff: String,
    end: String,
    start_trans: String,
    kickoff_trans: String,
    end_trans: String,
}

#[derive(Deserialize)]
struct Game {
    start_date: String,
}

#[derive(Deserialize)]
struct Play {
    wallclock: String,
}

#[get("/teams")]
async fn teams() -> impl Responder {

    let token = env::var("CFB_TOKEN").unwrap();

    let client = Client::new();
    let temp = client.get("https://api.collegefootballdata.com/teams/fbs?year=2022")
        .bearer_auth(token)
        .send().await;

    if let Ok(resp) = temp {
        HttpResponse::Ok().body(resp.text().await.unwrap())
    } else {
        HttpResponse::BadRequest().finish()
    }
}

#[get("/time")]
async fn game_time(info: web::Query<TimeReq>) -> impl Responder {

    let token = env::var("CFB_TOKEN").unwrap();

    let client = Client::new();
    let temp = client.get(format!(
        "https://api.collegefootballdata.com/games?year={}&week={}&seasonType=regular&team={}",
        info.year, info.week, info.team
    ))
        .bearer_auth(token.clone())
        .send().await;

    let mut response = TimeResp::default();

    if let Ok(resp) = temp {
        let game = resp.json::<Vec<Game>>().await;
        if let Ok(game) = game {
            if !game.is_empty() {
                let start_time =
                    OffsetDateTime::parse(&game.first().unwrap().start_date, &well_known::Rfc3339)
                        .expect("Failed to parse start date");
                let start_time_trans =
                    start_time.to_offset(UtcOffset::from_hms(info.offset, 0, 0).unwrap());

                response.team = info.team.clone();
                response.start = format!("{:0>2}:{:0>2}", start_time.hour(), start_time.minute());
                response.start_trans = format!(
                    "{:0>2}:{:0>2}",
                    start_time_trans.hour(),
                    start_time_trans.minute()
                );

                let temp = client.get(format!(
                    "https://api.collegefootballdata.com/plays?seasonType=regular&year={}&week={}&team={}",
                    info.year, info.week, info.team
                ))
                    .bearer_auth(token)
                    .send().await;

                if let Ok(resp) = temp {
                    let plays = resp.json::<Vec<Play>>().await;
                    if let Ok(plays) = plays {
                        let first = plays.first().unwrap();
                        let last = plays.last().unwrap();
                        let kickoff_time =
                            OffsetDateTime::parse(&first.wallclock, &well_known::Rfc3339)
                                .expect("Failed to parse kickoff time");
                        let kickoff_time_trans =
                            kickoff_time.to_offset(UtcOffset::from_hms(info.offset, 0, 0).unwrap());

                        response.kickoff =
                            format!("{:0>2}:{:0>2}", kickoff_time.hour(), kickoff_time.minute());
                        response.kickoff_trans = format!(
                            "{:0>2}:{:0>2}",
                            kickoff_time_trans.hour(),
                            kickoff_time_trans.minute()
                        );

                        let end_time = OffsetDateTime::parse(&last.wallclock, &well_known::Rfc3339)
                            .expect("Failed to parse end time");
                        let end_time_trans =
                            end_time.to_offset(UtcOffset::from_hms(info.offset, 0, 0).unwrap());

                        response.end = format!("{:0>2}:{:0>2}", end_time.hour(), end_time.minute());
                        response.end_trans = format!(
                            "{:0>2}:{:0>2}",
                            end_time_trans.hour(),
                            end_time_trans.minute()
                        );
                    }
                }

                HttpResponse::Ok().body(serde_json::to_string(&response).unwrap())
            } else {
                HttpResponse::Ok().body("")
            }
        } else {
            HttpResponse::Ok().body("")
        }
    } else {
        HttpResponse::Ok().body("")
    }
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {

    let email = "steviecook210@gmail.com";
    let domain = "18.191.220.43";

    //   Load keys
    // ==============================================
    // = IMPORTANT:                                 =
    // = This process has to be repeated            =
    // = before the certificate expires (< 90 days) =
    // ==============================================
    // Obtain TLS certificate
    let cert = gen_tls_cert(email, domain).await?;
    let mut ssl_builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())?;

    // Get and add private key
    let pkey_der = PKey::private_key_from_der(&cert.private_key_der()?)?;
    ssl_builder.set_private_key(&pkey_der)?;

    // Get and add certificate
    let cert_der = X509::from_der(&cert.certificate_der()?)?;
    ssl_builder.set_certificate(&cert_der)?;

    // Get and add intermediate certificate to the chain
    let icert_url = "https://letsencrypt.org/certs/lets-encrypt-r3.der";
    let icert_bytes = reqwest::get(icert_url).await?.bytes().await?;
    let intermediate_cert = X509::from_der(&icert_bytes)?;
    ssl_builder.add_extra_chain_cert(intermediate_cert)?;

    let srv = HttpServer::new(|| {

        let cors = Cors::default()
            .allow_any_origin()
            .allowed_methods(vec!["GET"])
            .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
            .allowed_header(header::CONTENT_TYPE)
            .max_age(3600);

        App::new().wrap(cors).service(teams).service(game_time)
    }).bind_openssl(("0.0.0.0", 8080), ssl_builder)?
        .run();

    let srv_handle = srv.handle();

    let _auto_shutdown_task = rt::spawn(async move {
        // Shutdown server every 4 weeks so that TLS certs can be regenerated if needed.
        // This is only appropriate in contexts like Kubernetes which can orchestrate restarts.
        rt::time::sleep(Duration::from_secs(60 * 60 * 24 * 28)).await;
        srv_handle.stop(true).await;
    });

    srv.await?;

    Ok(())
}

pub async fn gen_tls_cert(user_email: &str, user_domain: &str) -> anyhow::Result<Certificate> {
    // Create acme-challenge dir.
    fs::create_dir("./acme-challenge").unwrap();

    let domain = user_domain.to_string();

    // Create temporary Actix Web server for ACME challenge.
    let srv = HttpServer::new(|| {
        App::new().service(
            Files::new(
                // HTTP route
                "/.well-known/acme-challenge",
                // Server's dir
                "acme-challenge",
            )
                .show_files_listing(),
        )
    })
        .bind((domain, 80))?
        .shutdown_timeout(0)
        .run();

    let srv_handle = srv.handle();
    let srv_task = rt::spawn(srv);

    // Use DirectoryUrl::LetsEncryptStaging for dev/testing.
    let url = DirectoryUrl::LetsEncrypt;

    // Create a directory entrypoint.
    let dir = Directory::from_url(url)?;

    // Our contact addresses; note the `mailto:`
    let user_email_mailto: String = "mailto:{email}".replace("{email}", user_email);
    let contact = vec![user_email_mailto];

    // Generate a private key and register an account with our ACME provider.
    // We should write it to disk any use `load_account` afterwards.
    let acc = dir.register_account(contact.clone())?;

    // Load an account from string
    let privkey = acc.acme_private_key_pem()?;
    let acc = dir.load_account(&privkey, contact)?;

    // Order a new TLS certificate for the domain.
    let mut ord_new = acc.new_order(user_domain, &[])?;

    // If the ownership of the domain have already been
    // authorized in a previous order, we might be able to
    // skip validation. The ACME API provider decides.
    let ord_csr = loop {
        // Are we done?
        if let Some(ord_csr) = ord_new.confirm_validations() {
            break ord_csr;
        }

        // Get the possible authorizations (for a single domain
        // this will only be one element).
        let auths = ord_new.authorizations()?;

        // For HTTP, the challenge is a text file that needs to
        // be placed in our web server's root:
        //
        // <mydomain>/acme-challenge/<token>
        //
        // The important thing is that it's accessible over the
        // web for the domain we are trying to get a
        // certificate for:
        //
        // http://mydomain.io/.well-known/acme-challenge/<token>
        let chall = auths[0]
            .http_challenge()
            .ok_or_else(|| anyhow!("no HTTP challenge accessible"))?;

        // The token is the filename.
        let token = chall.http_token();

        // The proof is the contents of the file
        let proof = chall.http_proof()?;

        // Place the file/contents in the correct place.
        let path = format!("acme-challenge/{token}");
        fs::write(&path, &proof)?;

        // After the file is accessible from the web, the calls
        // this to tell the ACME API to start checking the
        // existence of the proof.
        //
        // The order at ACME will change status to either
        // confirm ownership of the domain, or fail due to the
        // not finding the proof. To see the change, we poll
        // the API with 5000 milliseconds wait between.
        chall.validate(Duration::from_millis(5000))?;

        // Update the state against the ACME API.
        ord_new.refresh()?;
    };

    // Ownership is proven. Create a private key for
    // the certificate. These are provided for convenience; we
    // could provide our own keypair instead if we want.
    let pkey_pri = create_p384_key()?;

    // Submit the CSR. This causes the ACME provider to enter a
    // state of "processing" that must be polled until the
    // certificate is either issued or rejected. Again we poll
    // for the status change.
    let ord_cert = ord_csr.finalize_pkey(pkey_pri, Duration::from_millis(5000))?;

    // Now download the certificate. Also stores the cert in
    // the persistence.
    let cert = ord_cert.download_cert()?;

    // Stop temporary server for ACME challenge
    srv_handle.stop(true).await;
    srv_task.await??;

    // Delete acme-challenge dir
    fs::remove_dir_all("./acme-challenge")?;

    Ok(cert)
}
