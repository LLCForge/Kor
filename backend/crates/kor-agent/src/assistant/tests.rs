use super::*;

fn setup() -> (
    tempfile::TempDir,
    Vec<Arc<dyn Tool>>,
    Vec<&'static str>,
    ToolCtx,
) {
    let dir = tempfile::tempdir().unwrap();
    let (tools, deferred) = tools(dir.path().join("assistant.json"), false);
    let ctx = ToolCtx::new(dir.path()).unwrap();
    (dir, tools, deferred, ctx)
}

fn tool<'a>(tools: &'a [Arc<dyn Tool>], name: &str) -> &'a Arc<dyn Tool> {
    tools.iter().find(|t| t.name() == name).unwrap()
}

#[test]
fn only_time_and_compose_load_up_front() {
    let (_d, tools, deferred, _c) = setup();
    assert_eq!(tools.len(), OPS.len());
    assert!(!deferred.contains(&"current_time") && !deferred.contains(&"compose_message"));
    assert!(deferred.contains(&"create_event") && deferred.contains(&"weather"));
    let section = deferred_section(&tools, &deferred);
    assert!(
        section.contains("- `weather`: Current weather"),
        "{section}"
    );
    assert!(!section.contains("`current_time`"));
    let (_, desktop) = super::tools(std::env::temp_dir().join("unused.json"), true);
    assert!(!desktop.contains(&"weather") && !desktop.contains(&"show_chart"));
    assert!(
        !desktop.contains(&"create_event")
            && !desktop.contains(&"search_events")
            && !desktop.contains(&"create_reminders")
    );
    assert!(
        desktop.contains(&"update_events")
            && desktop.contains(&"delete_reminders")
            && desktop.contains(&"list_calendars")
    );
}

#[tokio::test]
async fn cards_validate_and_carry_their_input() {
    let (_d, tools, _, ctx) = setup();
    let compose = json!({"kind": "email", "summary_title": "Toplantı", "variants": [
        {"label": "Resmi", "subject": "Yarın", "body": "Merhaba,\nyarın 10:00 uygun mu?"}
    ]});
    let t = tool(&tools, "compose_message");
    assert_eq!(t.summary(&compose), compose.to_string());
    let out = t.call(compose, &ctx).await;
    assert!(
        out.content
            .contains("--- Resmi ---\nSubject: Yarın\n\nMerhaba"),
        "{}",
        out.content
    );

    let chart = tool(&tools, "show_chart");
    let bad = chart
        .call(
            json!({"style": "pie", "series": [{"name": "a", "data": [1]}]}),
            &ctx,
        )
        .await;
    assert!(bad.is_error);
    let bad = chart
        .call(
            json!({"style": "bar", "series": [{"name": "a", "data": [1], "color": "red"}]}),
            &ctx,
        )
        .await;
    assert!(bad.is_error && bad.content.contains("#RRGGBB"));
    let ok = chart
        .call(
            json!({"style": "bar", "title": "Satış", "series": [{"name": "2026", "data": [3, 5]}]}),
            &ctx,
        )
        .await;
    assert_eq!(ok.content, "Chart shown: Satış (1 series, bar)");

    let widget = tool(&tools, "show_widget");
    assert!(
        widget
            .call(json!({"title": "x", "widget_code": "<p>hi</p>"}), &ctx)
            .await
            .is_error
    );

    let recipe = tool(&tools, "show_recipe")
        .call(json!({"title": "Çay", "ingredients": [{"id": "t", "name": "çay", "amount": 2, "unit": "tsp"}],
                     "steps": [{"id": "1", "title": "Demle", "content": "{t} ekle", "timer_seconds": 600}]}), &ctx)
        .await;
    assert!(
        recipe.content.contains("1. Demle [10 min]: 2 tsp çay ekle"),
        "{}",
        recipe.content
    );

    let map = tool(&tools, "show_map")
        .call(json!({"markers": [{"lat": 99, "lon": 0}]}), &ctx)
        .await;
    assert!(map.is_error);
    assert_eq!(
        tool(&tools, "weather").summary(&json!({"location": "Izmir"})),
        "Izmir"
    );
}

#[tokio::test]
async fn calendar_round_trip() {
    let (_d, tools, _, ctx) = setup();
    let create = tool(&tools, "create_event");
    assert_eq!(create.kind(), ToolKind::Edit);
    let out = create
        .call(json!({"title": "Diş hekimi", "start_time": "2026-11-12T14:30", "location": "Alsancak"}), &ctx)
        .await;
    assert_eq!(
        out.content,
        "Saved: [2] 2026-11-12 14:30 – 2026-11-12 15:30 Diş hekimi @ Alsancak"
    );
    let bad = create
        .call(
            json!({"title": "x", "start_time": "2026-11-12T14:30", "end_time": "2026-11-12T13:00"}),
            &ctx,
        )
        .await;
    assert!(bad.is_error);
    // Models often pass the calendar's name as its id.
    let named = create
        .call(
            json!({"title": "Tatil", "start_time": "2026-12-01", "all_day": true, "calendar_id": "personal"}),
            &ctx,
        )
        .await;
    assert!(!named.is_error, "{}", named.content);
    let unknown = create
        .call(
            json!({"title": "x", "start_time": "2026-12-02", "calendar_id": "Work"}),
            &ctx,
        )
        .await;
    assert!(unknown.is_error && unknown.content.contains("list_calendars"));

    let search = tool(&tools, "search_events");
    let nov = search
        .call(json!({"from": "2026-11-01", "to": "2026-12-01"}), &ctx)
        .await;
    assert!(
        nov.content.contains("Diş hekimi") && !nov.content.contains("Tatil"),
        "{}",
        nov.content
    );
    let text = search.call(json!({"text": "TATİL"}), &ctx).await;
    assert!(
        text.content.contains("[3] 2026-12-01 (all day) Tatil")
            || text.content.contains("No events"),
        "{}",
        text.content
    );

    let upd = tool(&tools, "update_events")
        .call(json!({"updates": [{"event_id": "2", "start_time": "2026-11-12T16:00", "end_time": "2026-11-12T17:00"}]}), &ctx)
        .await;
    assert!(
        upd.content.contains("16:00 – 2026-11-12 17:00"),
        "{}",
        upd.content
    );
    let missing = tool(&tools, "update_events")
        .call(json!({"updates": [{"event_id": "2", "title": "x"}, {"event_id": "404", "title": "y"}]}), &ctx)
        .await;
    assert!(missing.is_error);
    let still = search.call(json!({"text": "diş"}), &ctx).await;
    assert!(
        still.content.contains("Diş hekimi"),
        "a failed batch changes nothing: {}",
        still.content
    );

    let del = tool(&tools, "delete_events");
    assert_eq!(del.kind(), ToolKind::Execute);
    assert_eq!(
        del.call(json!({"event_ids": ["2", "3"]}), &ctx)
            .await
            .content,
        "Deleted 2 event(s)."
    );
    assert_eq!(search.call(json!({}), &ctx).await.content, "No events.");
}

#[tokio::test]
async fn reminders_round_trip() {
    let (_d, tools, _, ctx) = setup();
    let out = tool(&tools, "create_reminders")
        .call(json!({"reminders": [{"title": "Süt al"}, {"title": "Faturayı öde", "due": "2026-11-15T09:00", "notes": "elektrik"}]}), &ctx)
        .await;
    assert!(
        out.content.contains("Added: [2] [ ] Süt al"),
        "{}",
        out.content
    );
    let search = tool(&tools, "search_reminders");
    let all = search.call(json!({}), &ctx).await;
    assert!(
        all.content
            .starts_with("[3] [ ] Faturayı öde (due 2026-11-15 09:00) — elektrik"),
        "{}",
        all.content
    );
    tool(&tools, "update_reminders")
        .call(json!({"updates": [{"id": "2", "completed": true}]}), &ctx)
        .await;
    assert!(!search.call(json!({}), &ctx).await.content.contains("Süt"));
    assert!(
        search
            .call(json!({"include_completed": true}), &ctx)
            .await
            .content
            .contains("[x] Süt al")
    );
    assert_eq!(
        tool(&tools, "delete_reminders")
            .call(json!({"ids": ["2"]}), &ctx)
            .await
            .content,
        "Deleted 1 reminder(s)."
    );
}

#[test]
fn forecast_becomes_the_card_view() {
    let payload = json!({
        "current": {"temperature_2m": 18.26, "weather_code": 2, "precipitation": 0.0, "wind_speed_10m": 11.0},
        "daily": {"time": ["2026-11-10", "2026-11-11"], "weather_code": [61, 0],
                  "temperature_2m_max": [19.4, 21.0], "temperature_2m_min": [12.1, 11.0],
                  "precipitation_probability_max": [80, 5]}
    });
    let v = weather_view(&payload, "İzmir, Türkiye").unwrap();
    assert_eq!(v["current"]["temp_c"], 18.3);
    assert_eq!(v["current"]["temp_f"], 64.9);
    assert_eq!(v["current"]["label"], "Partly cloudy");
    assert_eq!(v["days"][0]["label"], "Rain");
    assert_eq!(v["days"][1]["precip_probability"], 5.0);
    assert!(weather_view(&json!({}), "x").is_err());
}

/// `cargo test -p kor-agent -- --ignored real_weather`
#[tokio::test]
#[ignore = "calls Open-Meteo"]
async fn real_weather_and_places() {
    let (_d, tools, _, ctx) = setup();
    let w = tool(&tools, "weather")
        .call(json!({"location": "Izmir", "days": 3}), &ctx)
        .await;
    assert!(!w.is_error, "{}", w.content);
    let v: Value = serde_json::from_str(&w.content).unwrap();
    assert_eq!(v["days"].as_array().unwrap().len(), 3);
    assert!(
        v["location_name"].as_str().unwrap().contains("zmir"),
        "{}",
        w.content
    );
    let p = tool(&tools, "search_places")
        .call(json!({"query": "Ankara"}), &ctx)
        .await;
    assert!(p.content.contains("lat 39."), "{}", p.content);
}

#[test]
fn query_values_are_percent_encoded() {
    assert_eq!(
        with_query(
            "https://x/s",
            &[("name", "İzmir & co".into()), ("count", "1".into())]
        ),
        "https://x/s?name=%C4%B0zmir%20%26%20co&count=1"
    );
}
