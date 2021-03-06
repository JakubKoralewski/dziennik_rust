//! MIT License
//! Copyright (c) 2019 Jakub Koralewski

extern crate actix_web;
extern crate listenfd;
extern crate pretty_env_logger;
extern crate env_logger;
#[macro_use] extern crate serde_derive;
extern crate sentry_actix;
#[macro_use] extern crate diesel;

use sentry::{Hub, Level};
use std::default::Default;
use sentry_actix::{SentryMiddleware, ActixWebHubExt};

#[allow(unused_imports)] // it's useful to have these in scope
use log::{debug, error, info, warn};
use env_logger::Target;

use listenfd::ListenFd;
use actix_web::{
    server, 
    App,
    http::Method,
    middleware,
    error,
    HttpRequest,
    HttpResponse,
    middleware::cors::Cors,
    actix::{
        SyncArbiter,
        Addr,
        System
    }
};
use std::env;
use dotenv::dotenv;

mod students;
mod login;
mod schema;
mod database;

#[derive(Deserialize, Serialize)]
struct JsonError {
    message: String,
}


/// Handles returning info to client about errors
/// regarding the json request body.
fn json_error_handler(err: error::JsonPayloadError, req: &HttpRequest<State>) -> error::Error {
    error!("Bad json data: {:?}", &err);
    let message = format!("{}", err);
    
    let hub = Hub::from_request(req);
    hub.capture_message(message.as_str(), Level::Error);

    let description = JsonError{message};
    error::InternalError::from_response(
        err, HttpResponse::BadRequest().json(description)
    ).into()
}

/// Handles returning info to client about errors
/// regarding the id supplied in the path.
fn path_error_handler(err: serde::de::value::Error, req: &HttpRequest<State>) -> error::Error {
    error!("Bad path id: {:?}", &err);
    
    let message = format!("{}", err);
    
    let hub = Hub::from_request(req);
    hub.capture_message(message.as_str(), Level::Error);

    let description = JsonError{message};
    error::InternalError::from_response(
        err, HttpResponse::BadRequest().json(description)
    ).into()
}

pub struct State {
    pub db: Addr<database::Database>
}

fn main() {
    /* Environment variables */
    dotenv().ok();
    env::set_var("RUST_BACKTRACE", "1");
    const RUST_LOG: &'static str = "debug, actix_web=debug";
    env::set_var("RUST_LOG", &RUST_LOG);
    let mut log_builder = pretty_env_logger::formatted_builder();

    /* Sentry */
    let _sentry;
    if let Ok(dsn) = env::var("SENTRY_DSN") {
        _sentry = sentry::init(dsn);
        sentry::integrations::env_logger::init(
            Some(log_builder.parse_filters(&RUST_LOG).target(Target::Stdout).build()),
            Default::default()
        );
        sentry::integrations::panic::register_panic_handler();
    }
    
    let ip: &str = "0.0.0.0";
    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .expect("PORT must be a number");
    

    debug!("Listening on {}:{}", &ip, &port);

    /* Setup autoreload */
    let mut listenfd = ListenFd::from_env();

    /* Database */
    let sys = System::new("dziennik");
    let pool = database::pool();
    let addr = SyncArbiter::start(12, move || database::Database(pool.clone()));

    /* Start server */
    let mut server = server::new(move || {
        App::with_state(State {
            db: addr.clone()
        })
            .middleware(SentryMiddleware::new())
            .middleware(middleware::Logger::default())
            .prefix("/api")
            .configure(|app| {
                Cors::for_app(app)
                    .allowed_methods(vec!["GET", "POST", "PUT", "DELETE"])
                    .max_age(3600)
                    .resource("/students", |r| {
                        r.method(Method::POST).with_async_config(students::create, |cfg| {
                            (cfg.0).1.error_handler(&json_error_handler);
                        });
                        r.method(Method::GET).a(students::read);
                    })
                    .resource("/students/{id}", |r| {       // register resource
                        r.method(Method::PUT).with_async_config(students::update, |cfg| {
                            (cfg.0).1.error_handler(&path_error_handler);
                            (cfg.0).2.error_handler(&json_error_handler);
                        });
                        r.method(Method::DELETE).with_async_config(students::delete, |cfg| {
                            (cfg.0).1.error_handler(&path_error_handler);
                        });
                    })
                    .resource("/login", |r| {       // register resource
                        r.method(Method::POST).with_async_config(login::login, |cfg| {
                            (cfg.0).1.error_handler(&json_error_handler);
                        })
                    })
                    .register()
            })
    });

    // For auto-reload
    server = if let Some(l) = listenfd.take_tcp_listener(0).unwrap() {
        server.listen(l)
    } else {
        server.bind((ip, port)).unwrap()
    };

    server.run();
    sys.run();
} 



