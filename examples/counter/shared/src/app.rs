use chrono::{serde::ts_milliseconds_option::deserialize as ts_milliseconds_option, DateTime, Utc};
use crux_core::{
    render::{self, Render},
    Command,
};
use crux_http::command::Http;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::sse::ServerSentEvents;

const API_URL: &str = "https://crux-counter.fly.dev";

// ANCHOR: model
#[derive(Default, Serialize)]
pub struct Model {
    count: Count,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq, Eq)]
pub struct Count {
    value: isize,
    #[serde(deserialize_with = "ts_milliseconds_option")]
    updated_at: Option<DateTime<Utc>>,
}
// ANCHOR_END: model

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ViewModel {
    pub text: String,
    pub confirmed: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Event {
    // events from the shell
    Get,
    Increment,
    Decrement,
    StartWatch,

    // events local to the core
    #[serde(skip)]
    Set(crux_http::Result<crux_http::Response<Count>>),
    #[serde(skip)]
    Update(Count),
}

#[cfg_attr(feature = "typegen", derive(crux_core::macros::Export))]
#[derive(crux_core::macros::Effect)]
pub struct Capabilities {
    pub render: Render<Event>,
    pub http: crux_http::Http<Event>,
    pub sse: ServerSentEvents<Event>,
}

#[derive(Default)]
pub struct App;

impl crux_core::App for App {
    type Model = Model;
    type Event = Event;
    type ViewModel = ViewModel;
    type Capabilities = Capabilities;
    type Effect = Effect;

    // During the migration to the new Command API, the `update` method
    // still requires the old `Capabilities` type. This will be removed in
    // an upcoming release.
    // In the meantime, we can delegate to our own `update` function,
    // so that we can test the logic without the need for AppTester.
    fn update(
        &self,
        msg: Self::Event,
        model: &mut Self::Model,
        _caps: &Self::Capabilities,
    ) -> Command<Effect, Event> {
        self.update(msg, model)
    }

    fn view(&self, model: &Self::Model) -> Self::ViewModel {
        let suffix = match model.count.updated_at {
            None => " (pending)".to_string(),
            Some(d) => format!(" ({d})"),
        };

        Self::ViewModel {
            text: model.count.value.to_string() + &suffix,
            confirmed: model.count.updated_at.is_some(),
        }
    }
}

impl App {
    fn update(&self, msg: Event, model: &mut Model) -> Command<Effect, Event> {
        match msg {
            Event::Get => Http::get(API_URL)
                .expect_json()
                .build()
                .then_send(Event::Set),
            Event::Set(Ok(mut response)) => {
                let count = response.take_body().unwrap();
                Command::event(Event::Update(count))
            }
            Event::Set(Err(e)) => {
                panic!("Oh no something went wrong: {e:?}");
            }
            Event::Update(count) => {
                model.count = count;
                render::render()
            }
            Event::Increment => {
                // optimistic update
                model.count = Count {
                    value: model.count.value + 1,
                    updated_at: None,
                };

                // real update
                let base = Url::parse(API_URL).unwrap();
                let url = base.join("/inc").unwrap();
                Command::all([
                    Http::post(url).expect_json().build().then_send(Event::Set),
                    render::render(),
                ])
            }
            Event::Decrement => {
                // optimistic update
                model.count = Count {
                    value: model.count.value - 1,
                    updated_at: None,
                };

                // real update
                let base = Url::parse(API_URL).unwrap();
                let url = base.join("/dec").unwrap();
                Command::all([
                    Http::post(url).expect_json().build().then_send(Event::Set),
                    render::render(),
                ])
            }
            Event::StartWatch => {
                let base = Url::parse(API_URL).unwrap();
                let url = base.join("/sse").unwrap();
                ServerSentEvents::get_json(url, Event::Update)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{App, Event, Model};
    use crate::{capabilities::sse::SseRequest, Count, Effect};

    use crux_core::{assert_effect, App as _};
    use crux_http::{
        protocol::{HttpRequest, HttpResponse, HttpResult},
        testing::ResponseBuilder,
    };

    // ANCHOR: simple_tests
    /// Test that a `Get` event causes the app to fetch the current
    /// counter value from the web API
    #[test]
    fn get_counter() {
        let app = App::default();
        let mut model = Model::default();

        let mut cmd = app.update(Event::Get, &mut model);

        // check that an HTTP request was emitted,
        // capturing the request in the process
        let mut request = cmd.effects().next().unwrap().expect_http();

        // check that the request is a GET to the correct URL
        let actual = &request.operation;
        let expected = &HttpRequest::get("https://crux-counter.fly.dev/").build();
        assert_eq!(actual, expected);

        // resolve the request with a simulated response from the web API
        request
            .resolve(HttpResult::Ok(
                HttpResponse::ok()
                    .body(r#"{ "value": 1, "updated_at": 1672531200000 }"#)
                    .build(),
            ))
            .unwrap();

        // check that an (internal) event was emitted to update the model
        let actual = cmd.events().next().unwrap();
        let expected = Event::Set(Ok(ResponseBuilder::ok()
            .body(Count {
                value: 1,
                updated_at: Some(Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap()),
            })
            .build()));
        assert_eq!(actual, expected);
    }

    /// Test that a `Set` event causes the app to update the model
    #[test]
    fn set_counter() {
        let app = App::default();
        let mut model = Model::default();

        // send a `Set` event (containing the HTTP response) to the app
        let mut cmd = app.update(
            Event::Set(Ok(ResponseBuilder::ok()
                .body(Count {
                    value: 1,
                    updated_at: Some(Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap()),
                })
                .build())),
            &mut model,
        );

        // submit the generated `Update` event back to the app
        let event = cmd.events().next().unwrap();
        let mut cmd = app.update(event, &mut model);

        // check that the app asked the shell to render
        assert_effect!(cmd, Effect::Render(_));

        // check that the view has been updated correctly
        insta::assert_yaml_snapshot!(app.view(&model), @r###"
        ---
        text: "1 (2023-01-01 00:00:00 UTC)"
        confirmed: true
        "###);
    }
    // ANCHOR_END: simple_tests

    // Test that an `Increment` event causes the app to increment the counter
    #[test]
    fn increment_counter() {
        let app = App::default();

        // set up our initial model as though we've previously fetched the counter
        let mut model = Model {
            count: Count {
                value: 1,
                updated_at: Some(Utc.with_ymd_and_hms(2022, 12, 31, 23, 59, 0).unwrap()),
            },
        };

        // send an `Increment` event to the app
        let mut cmd = app.update(Event::Increment, &mut model);

        // the first effect should be an Http post
        let mut request = cmd.effects().next().unwrap().expect_http();
        let actual = &request.operation;
        let expected = &HttpRequest::post("https://crux-counter.fly.dev/inc").build();
        assert_eq!(actual, expected);

        // we should also get a render effect
        assert_effect!(cmd, Effect::Render(_));

        // we are expecting our model to be updated "optimistically" before the
        // HTTP request completes, so the value should have been updated
        // but not the timestamp
        assert_eq!(
            model.count,
            Count {
                value: 2,
                updated_at: None
            }
        );

        // resolve the request with a simulated response from the web API
        request
            .resolve(HttpResult::Ok(
                HttpResponse::ok()
                    .body(r#"{ "value": 2, "updated_at": 1672531200000 }"#)
                    .build(),
            ))
            .unwrap();

        // this should generate a Set event
        let event = cmd.events().next().unwrap();
        assert!(matches!(event, Event::Set(_)));

        // let's send that back to the app
        let mut cmd = app.update(event, &mut model);

        // this should generate an Update event
        let event = cmd.events().next().unwrap();
        assert_eq!(
            event,
            Event::Update(Count {
                value: 2,
                updated_at: Some(Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap()),
            })
        );

        // let's send that back to the app
        let mut cmd = app.update(event, &mut model);

        // check that the app asked the shell to render
        assert_effect!(cmd, Effect::Render(_));

        // check that the model has been updated correctly
        insta::assert_yaml_snapshot!(model, @r###"
        ---
        count:
          value: 2
          updated_at: "2023-01-01T00:00:00Z"
        "###);
    }

    /// Test that a `Decrement` event causes the app to decrement the counter
    #[test]
    fn decrement_counter() {
        let app = App::default();

        // set up our initial model as though we've previously fetched the counter
        let mut model = Model {
            count: Count {
                value: 0,
                updated_at: Some(Utc.with_ymd_and_hms(2022, 12, 31, 23, 59, 0).unwrap()),
            },
        };

        // send a `Decrement` event to the app
        let mut update = app.update(Event::Decrement, &mut model);

        // the first effect should be an Http post
        let mut request = update.effects().next().unwrap().expect_http();
        let actual = &request.operation;
        let expected = &HttpRequest::post("https://crux-counter.fly.dev/dec").build();
        assert_eq!(actual, expected);

        // we should also get a render effect
        assert_effect!(update, Effect::Render(_));

        // we are expecting our model to be updated "optimistically" before the
        // HTTP request completes, so the value should have been updated
        // but not the timestamp
        assert_eq!(
            model.count,
            Count {
                value: -1,
                updated_at: None
            }
        );

        // resolve the request with a simulated response from the web API
        request
            .resolve(HttpResult::Ok(
                HttpResponse::ok()
                    .body(r#"{ "value": -1, "updated_at": 1672531200000 }"#)
                    .build(),
            ))
            .unwrap();

        // this should generate a Set event
        let event = update.events().next().unwrap();
        assert!(matches!(event, Event::Set(_)));

        // let's send that back to the app
        let mut update = app.update(event, &mut model);

        // this should generate an Update event
        let event = update.events().next().unwrap();
        assert_eq!(
            event,
            Event::Update(Count {
                value: -1,
                updated_at: Some(Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap()),
            })
        );

        // let's send that back to the app
        let mut update = app.update(event, &mut model);

        // check that the app asked the shell to render
        assert_effect!(update, Effect::Render(_));

        // check that the model has been updated correctly
        insta::assert_yaml_snapshot!(model, @r###"
        ---
        count:
          value: -1
          updated_at: "2023-01-01T00:00:00Z"
        "###);
    }

    #[test]
    fn get_sse() {
        let app = App::default();
        let mut model = Model::default();

        let mut cmd = app.update(Event::StartWatch, &mut model);

        let request = cmd.effects().next().unwrap().expect_sse();

        assert_eq!(
            request.operation,
            SseRequest {
                url: "https://crux-counter.fly.dev/sse".to_string(),
            }
        );
    }

    #[test]
    fn set_sse() {
        let app = App::default();
        let mut model = Model::default();

        let count = Count {
            value: 1,
            updated_at: Some(Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap()),
        };
        let event = Event::Update(count);

        let mut cmd = app.update(event, &mut model);

        assert_effect!(cmd, Effect::Render(_));

        // check that the model has been updated correctly
        insta::assert_yaml_snapshot!(model, @r###"
        ---
        count:
          value: 1
          updated_at: "2023-01-01T00:00:00Z"
        "###);
    }
}
