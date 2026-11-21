use chrono::{Duration, TimeZone, Utc};
use domain::scheduled_event::{EventKind, EventStatus, ScheduledEventDetails};

fn details(kind: EventKind) -> ScheduledEventDetails {
    let start = Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap();
    ScheduledEventDetails {
        name: "Meetup".into(),
        description: None,
        start,
        end: Some(start + Duration::hours(1)),
        kind,
        status: EventStatus::Scheduled,
    }
}

#[test]
fn external_events_need_a_nonempty_location_and_end_after_start() {
    let mut event = details(EventKind::External {
        location: " ".into(),
    });
    assert!(event.validate().is_err());
    event.kind = EventKind::External {
        location: "Park".into(),
    };
    event.end = None;
    assert!(event.validate().is_err());
    event.end = Some(event.start);
    assert!(event.validate().is_err());
    event.end = Some(event.start + Duration::minutes(1));
    assert!(event.validate().is_ok());
}

#[test]
fn event_names_and_descriptions_obey_wire_limits() {
    let mut event = details(EventKind::External {
        location: "Park".into(),
    });
    event.name = " ".into();
    assert!(event.validate().is_err());
    event.name = "a".repeat(101);
    assert!(event.validate().is_err());
    event.name = "Meetup".into();
    event.description = Some("a".repeat(1001));
    assert!(event.validate().is_err());
}

#[test]
fn only_valid_lifecycle_transitions_are_allowed() {
    assert!(EventStatus::Scheduled.can_transition_to(EventStatus::Active));
    assert!(EventStatus::Scheduled.can_transition_to(EventStatus::Canceled));
    assert!(EventStatus::Active.can_transition_to(EventStatus::Completed));
    assert!(!EventStatus::Completed.can_transition_to(EventStatus::Active));
    assert!(!EventStatus::Canceled.can_transition_to(EventStatus::Scheduled));
}
