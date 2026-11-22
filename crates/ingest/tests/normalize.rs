//! Normalize tests over synthetic payloads (fake ids and values, call-sign names).

use serde_json::json;
use withings_ingest::normalize;

#[test]
fn a_full_weigh_in_maps_every_measure() {
    let g = json!({
        "grpid": 111, "date": 1710000000, "category": 1, "attrib": 0,
        "deviceid": "dev-alpha", "model": 5, "modified": 1710000100,
        "measures": [
            {"value": 7234, "type": 1, "unit": -2, "algo": 3, "fm": 1},
            {"value": 1820, "type": 6, "unit": -1},
            {"value": 62,   "type": 11, "unit": 0}
        ]
    });
    let mg = normalize::group(&g).unwrap();
    assert_eq!(mg.grpid, 111);
    assert_eq!(mg.measures.len(), 3);
    assert_eq!(mg.person_id, None);
    assert_eq!(mg.measures[0].value, 7234);
    assert_eq!(mg.measures[0].algo, Some(3));
    assert_eq!(mg.category, Some(1));
}

#[test]
fn an_unknown_type_code_is_kept() {
    let g = json!({
        "grpid": 1, "date": 1710000000,
        "measures": [{"value": 5, "type": 9999, "unit": 0}]
    });
    let mg = normalize::group(&g).unwrap();
    assert_eq!(mg.measures[0].type_code, 9999);
}

#[test]
fn the_raw_payload_is_kept_whole() {
    let g = json!({"grpid": 2, "date": 1710000000, "measures": [], "brand": 7});
    let mg = normalize::group(&g).unwrap();
    assert_eq!(mg.raw.get("brand").and_then(|v| v.as_i64()), Some(7));
}

#[test]
fn a_missing_required_field_is_a_shape_error_naming_it() {
    let g = json!({"date": 1710000000, "measures": []});
    let err = normalize::group(&g).unwrap_err();
    assert!(err.0.contains("grpid"), "{err:?}");
}

#[test]
fn a_person_is_not_owner_until_the_runner_says_so() {
    let u = json!({"id": 7, "shortname": "ALPHA", "isshared": 0});
    let p = normalize::person(&u).unwrap();
    assert_eq!(p.id, 7);
    assert_eq!(p.label.as_deref(), Some("ALPHA"));
    assert!(!p.is_owner);
    assert!(!p.shared);
}

#[test]
fn a_shared_profile_is_flagged_shared() {
    let u = json!({"id": 8, "firstname": "BRAVO", "isshared": 1});
    let p = normalize::person(&u).unwrap();
    assert!(p.shared);
    assert_eq!(p.label.as_deref(), Some("BRAVO"));
}

#[test]
fn a_deleted_group_is_flagged() {
    let g = json!({"grpid": 3, "date": 1710000000, "measures": [], "deleted": 1});
    assert!(normalize::group_is_deleted(&g));
    let live = json!({"grpid": 4, "date": 1710000000, "measures": []});
    assert!(!normalize::group_is_deleted(&live));
}

#[test]
fn a_health_indicator_score_maps() {
    let s = json!({"scoreid": 55, "score_type": "vascular_age", "date": 1710000000, "score": 42.0, "status_classification": "good"});
    let sc = normalize::score("healthindicator", &s).unwrap();
    assert_eq!(sc.id, 55);
    assert_eq!(sc.source, "healthindicator");
    assert_eq!(sc.score_type.as_deref(), Some("vascular_age"));
    assert_eq!(sc.score, Some(42.0));
    assert_eq!(sc.person_id, None);
}

#[test]
fn a_vitality_score_uses_its_own_id_key() {
    let s = json!({"vitalityid": 77, "date": 1710000000, "score": 9.0});
    let sc = normalize::score("vitality", &s).unwrap();
    assert_eq!(sc.id, 77);
    assert_eq!(sc.source, "vitality");
}

#[test]
fn a_trajectory_maps() {
    let t = json!({"trajectoryid": 3, "type": "weight", "active": true});
    let tr = normalize::trajectory(&t).unwrap();
    assert_eq!(tr.id, 3);
    assert_eq!(tr.kind.as_deref(), Some("weight"));
    assert_eq!(tr.active, Some(true));
}
