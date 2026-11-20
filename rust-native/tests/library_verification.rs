use rustcast_core::{validate, SearchIndex};
use ruycast_native::templates::{Context, Limits, Plan};
use serde_json::json;

#[test]
fn original_private_library_aggregation_renders_templates_and_checks_keywords() {
    let mut input = rustcast_core::default_config();
    input["commands"] = json!([
        {
            "id": "url.fields",
            "title": "Documentation",
            "kind": "url",
            "target": "https://example.test/search?q={query default=\"ruycast\"}&place={place options=\"Amsterdam, Utrecht\"}",
            "enabled": false
        },
        {
            "id": "snippet.fields",
            "title": "Greeting",
            "kind": "snippet",
            "target": "Hello {name}{clipboard}{date}",
            "keyword": "hello"
        },
        {
            "id": "url.no-fields",
            "title": "Simple URL",
            "kind": "url",
            "target": "https://example.test/simple"
        },
        {
            "id": "snippet.no-fields",
            "title": "Plain text",
            "kind": "snippet",
            "target": "Plain text"
        },
        {
            "id": "app.ignored",
            "title": "Fixture app",
            "kind": "app",
            "target": "/Applications/Fixture.app"
        }
    ]);
    let config = validate(&input).unwrap();
    let index = SearchIndex::new(&config.commands);
    let context = Context {
        clipboard: Some("Fixture"),
        now: 0.0,
        locale: Some("en_GB"),
        time_zone: Some("UTC"),
    };
    let mut rendered_ids = Vec::new();
    let mut keyword_ids = Vec::new();

    for command in &config.commands {
        if command.kind != "snippet" && command.kind != "url" {
            continue;
        }
        let plan = Plan::parse(command, Limits::application()).unwrap();
        let values: Vec<(String, String)> = plan
            .fields()
            .iter()
            .map(|field| {
                let value = if !field.default_value.is_empty() {
                    field.default_value.clone()
                } else if let Some(option) = field.options.first() {
                    option.clone()
                } else {
                    "Fixture".into()
                };
                (field.id.clone(), value)
            })
            .collect();
        let (rendered, _) = plan.render(&values, &context).unwrap();
        rendered_ids.push(command.id.as_str());

        if let Some(keyword) = &command.keyword {
            let matches_own_command = index
                .search(keyword)
                .iter()
                .any(|&position| config.commands[position].id == command.id);
            assert!(matches_own_command, "keyword for {}", command.id);
            keyword_ids.push(command.id.as_str());
        }
        match command.id.as_str() {
            "url.fields" => assert_eq!(rendered, "https://example.test/search?q=ruycast&place=Amsterdam"),
            "snippet.fields" => assert_eq!(rendered, "Hello FixtureFixture1970-01-01"),
            "url.no-fields" => assert_eq!(rendered, "https://example.test/simple"),
            "snippet.no-fields" => assert_eq!(rendered, "Plain text"),
            _ => panic!("unexpected fixture command {}", command.id),
        }
    }

    assert_eq!(rendered_ids, ["url.fields", "snippet.fields", "url.no-fields", "snippet.no-fields"]);
    assert_eq!(keyword_ids, ["snippet.fields"]);
}
