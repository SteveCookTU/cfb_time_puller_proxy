use std::env;
use std::fs::File;
use std::io::BufReader;
use actix_cors::Cors;
use actix_web::{get, web, HttpServer, Responder, App, HttpResponse};
use actix_web::http::header;
use reqwest::Client;
use rustls::{Certificate, PrivateKey, ServerConfig};
use rustls_pemfile::{certs, pkcs8_private_keys};
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
        println!("Failed to get response");
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

fn load_rustls_config() -> ServerConfig {
    // init server config builder with safe defaults
    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth();

    // load TLS key/cert files
    let cert_file = &mut BufReader::new(File::open("cert.pem").unwrap());
    let key_file = &mut BufReader::new(File::open("key.pem").unwrap());

    // convert files to key/cert objects
    let cert_chain = certs(cert_file)
        .unwrap()
        .into_iter()
        .map(Certificate)
        .collect();
    let mut keys: Vec<PrivateKey> = pkcs8_private_keys(key_file)
        .unwrap()
        .into_iter()
        .map(PrivateKey)
        .collect();

    // exit if no keys could be parsed
    if keys.is_empty() {
        eprintln!("Could not locate PKCS 8 private keys.");
        std::process::exit(1);
    }

    config.with_single_cert(cert_chain, keys.remove(0)).unwrap()
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {

    let config = load_rustls_config();

    HttpServer::new(|| {

        let cors = Cors::default()
            .allow_any_origin()
            .allowed_methods(vec!["GET"])
            .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
            .allowed_header(header::CONTENT_TYPE)
            .max_age(3600);

        App::new().wrap(cors).service(teams).service(game_time)
    }).bind_rustls(("0.0.0.0", 8080), config)?
        .run().await

}
