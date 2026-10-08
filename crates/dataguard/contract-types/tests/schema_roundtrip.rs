//! For each contract: every generated value validates against the real
//! schema and survives serialize then deserialize unchanged.
//!
//! Default proptest config: 256 cases per test.

mod common;

use common::{
    arb_command_result, arb_impact_plan, arb_message, arb_queue_entry, assert_valid, g_validator,
    h_validator, j_validator, l_validator,
};
use dataguard_contract_types::{CommandResult, ImpactPlan, Message, QueueEntry};
use proptest::prelude::*;

proptest! {
    #[test]
    fn g_queue_entry(entry in arb_queue_entry()) {
        let doc = serde_json::to_value(&entry).unwrap();
        assert_valid(&g_validator(), &doc);
        let back: QueueEntry = serde_json::from_value(doc).unwrap();
        prop_assert_eq!(back, entry);
    }

    #[test]
    fn h_command_result(result in arb_command_result()) {
        let doc = serde_json::to_value(&result).unwrap();
        assert_valid(&h_validator(), &doc);
        let back: CommandResult = serde_json::from_value(doc).unwrap();
        prop_assert_eq!(back, result);
    }

    #[test]
    fn j_impact_plan(plan in arb_impact_plan()) {
        let doc = serde_json::to_value(&plan).unwrap();
        assert_valid(&j_validator(), &doc);
        let back: ImpactPlan = serde_json::from_value(doc).unwrap();
        prop_assert_eq!(back, plan);
    }

    #[test]
    fn l_message(message in arb_message()) {
        let doc = serde_json::to_value(&message).unwrap();
        assert_valid(&l_validator(), &doc);
        let back: Message = serde_json::from_value(doc).unwrap();
        prop_assert_eq!(back, message);
    }
}
