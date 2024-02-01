mod shared {
    use crux_core::macros::{Effect, Export};
    use crux_core::render::Render;
    use crux_http::Http;
    use serde::{Deserialize, Serialize};

    #[derive(Default)]
    pub(crate) struct App;

    #[derive(Serialize, Deserialize)]
    pub enum Event {
        Get,
        GetJson,

        // events local to the core
        #[serde(skip)]
        Set(crux_http::Result<crux_http::Response<Vec<u8>>>),
        #[serde(skip)]
        SetJson(crux_http::Result<crux_http::Response<String>>),
    }

    #[derive(Default, Serialize, Deserialize)]
    pub struct Model {
        pub status: u16,
        pub body: Vec<u8>,
        pub json_body: String,
    }

    #[derive(Serialize, Deserialize, Default)]
    pub struct ViewModel {
        pub result: String,
    }

    impl crux_core::App for App {
        type Event = Event;
        type Model = Model;
        type ViewModel = ViewModel;

        type Capabilities = Capabilities;

        fn update(&self, event: Event, model: &mut Model, caps: &Capabilities) {
            match event {
                Event::Get => {
                    caps.http.get("http://example.com").send(Event::Set);
                }
                Event::GetJson => {
                    caps.http
                        .get("http://example.com")
                        .expect_json::<String>()
                        .send(Event::SetJson);
                }
                Event::Set(response) => {
                    let mut response = response.unwrap();
                    model.status = response.status().into();
                    model.body = response.take_body().unwrap();
                    caps.render.render()
                }
                Event::SetJson(response) => {
                    model.json_body = response.unwrap().take_body().unwrap();
                    caps.render.render()
                }
            }
        }

        fn view(&self, model: &Self::Model) -> Self::ViewModel {
            ViewModel {
                result: format!(
                    "Status: {}, Body: {}, Json Body: {}",
                    model.status,
                    String::from_utf8_lossy(&model.body),
                    &model.json_body
                ),
            }
        }
    }

    #[derive(Effect, Export)]
    pub(crate) struct Capabilities {
        pub http: Http<Event>,
        pub render: Render<Event>,
    }
}

mod shell {
    use super::shared::{App, Effect, Event};
    use anyhow::Result;
    use crux_core::Core;
    use crux_http::protocol::{HttpRequest, HttpResponse};
    use std::collections::VecDeque;

    enum Task {
        Event(Event),
        Effect(Effect),
    }

    pub(crate) fn run(core: &Core<Effect, App>, event: Event) -> Result<Vec<HttpRequest>> {
        let mut queue: VecDeque<Task> = VecDeque::new();

        queue.push_back(Task::Event(event));

        let mut received: Vec<HttpRequest> = vec![];

        while !queue.is_empty() {
            let task = queue.pop_front().expect("an event");

            match task {
                Task::Event(event) => {
                    enqueue_effects(&mut queue, core.process_event(event));
                }
                Task::Effect(effect) => match effect {
                    Effect::Render(_) => (),
                    Effect::Http(mut request) => {
                        let http_request = &request.operation;

                        received.push(http_request.clone());
                        let response = HttpResponse::ok().json("Hello").build();

                        enqueue_effects(&mut queue, core.resolve(&mut request, response));
                    }
                },
            };
        }

        Ok(received)
    }

    fn enqueue_effects(queue: &mut VecDeque<Task>, effects: Vec<Effect>) {
        queue.append(&mut effects.into_iter().map(Task::Effect).collect())
    }
}

mod tests {
    use crate::{
        shared::{App, Effect, Event},
        shell::run,
    };
    use anyhow::Result;
    use crux_core::{typegen::TypeGen, Core};
    use crux_http::protocol::HttpRequest;

    #[test]
    pub fn test_http() -> Result<()> {
        let core: Core<Effect, App> = Core::default();

        let received = run(&core, Event::Get)?;

        assert_eq!(
            received,
            vec![HttpRequest::get("http://example.com/").build()]
        );

        assert_eq!(
            core.view().result,
            "Status: 200, Body: \"Hello\", Json Body: "
        );
        Ok(())
    }

    #[test]
    pub fn test_http_json() -> Result<()> {
        let core: Core<Effect, App> = Core::default();

        let received = run(&core, Event::GetJson)?;

        assert_eq!(
            received,
            vec![HttpRequest::get("http://example.com/").build()]
        );
        assert_eq!(core.view().result, "Status: 0, Body: , Json Body: Hello");
        Ok(())
    }

    #[test]
    fn generate_types() {
        let mut gen = TypeGen::new();

        gen.register_app::<App>().unwrap();

        let registry = match gen.state {
            crux_core::typegen::State::Registering(tracer, _) => {
                tracer.registry().expect("Should get registry")
            }
            crux_core::typegen::State::Generating(_) => {
                panic!("Expected to still be in registering stage")
            }
        };

        assert!(registry.contains_key("Event"));
        assert!(registry.contains_key("ViewModel"));

        assert!(registry.contains_key("Effect"));
        assert!(registry.contains_key("RenderOperation"));
        assert!(registry.contains_key("HttpRequest"));
        assert!(registry.contains_key("HttpResponse"));
    }
}
