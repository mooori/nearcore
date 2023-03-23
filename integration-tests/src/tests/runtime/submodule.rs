use crate::node::{setup_runtime_node_with_contract, Node};
use near_primitives::types::AccountId;
use near_primitives::views::FinalExecutionStatus;
use testlib::runtime_utils::alice_account;

/// Max prepaid amount of gas.
const MAX_GAS: u64 = 300_000_000_000_000;

fn test_contract_account() -> AccountId {
    format!("test-contract.{}", alice_account().as_str()).parse().unwrap()
}

#[test]
fn test_deploy_submodule_action() {
    let wasm_binary = near_test_contracts::rs_contract();
    let node = setup_runtime_node_with_contract(test_contract_account(), wasm_binary);

    let submodule_key = b"submodule1".to_vec();
    let submodule_code = near_test_contracts::trivial_contract().to_vec();

    // Deploy submodule.
    let tx_result = node
        .user()
        .deploy_submodule(test_contract_account(), submodule_key.clone(), submodule_code.clone())
        .expect("Transaction that deploys submodule should succeed");
    assert_eq!(tx_result.status, FinalExecutionStatus::SuccessValue(Vec::new()));

    // Retrieve the submodule to verify it was deployed.
    let tx_result = node
        .user()
        .function_call(
            alice_account(),
            test_contract_account(),
            "get_submodule",
            submodule_key,
            MAX_GAS,
            0,
        )
        .expect("Transaction that gets submodule should succeed");
    assert_eq!(tx_result.status, FinalExecutionStatus::SuccessValue(submodule_code));
}
