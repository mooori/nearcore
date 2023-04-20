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

#[test]
fn test_submodule_execution_no_resume() {
    let wasm_binary = near_test_contracts::rs_contract();
    let node = setup_runtime_node_with_contract(test_contract_account(), wasm_binary);

    let submodule_key = b"submodule1".to_vec();
    let submodule_code = submodule_return_int_no_resume();

    // Deploy submodule.
    let tx_result = node
        .user()
        .deploy_submodule(test_contract_account(), submodule_key.clone(), submodule_code.clone())
        .expect("Transaction that deploys submodule should succeed");
    assert_eq!(tx_result.status, FinalExecutionStatus::SuccessValue(Vec::new()));

    // Call a contract method that executes the submodule and returns its return value (i.e. 42).
    let tx_result = node
        .user()
        .function_call(
            alice_account(),
            test_contract_account(),
            "execute_submodule_no_resume",
            submodule_key,
            MAX_GAS,
            0,
        )
        .expect("Transaction that executes submodule should succeed");
    let expected_bytes = 42u64.to_le_bytes().to_vec();
    assert_eq!(tx_result.status, FinalExecutionStatus::SuccessValue(expected_bytes));
}

/// Returns the WebAssembly binary of a submodule that returns 42 (and does nothing else).
fn submodule_return_int_no_resume() -> Vec<u8> {
    wat::parse_str(
        r#"
            (module
                (type $t_env_return_value (func (param i64 i64)))
                (type $t_main (func))

                (import "env" "return_value" (func $env.return_value (type $t_env_return_value)))

                (memory 1)

                ;; Passes 42 to `return_value`.
                (func $main (export "main") (type $t_main)
                    ;; Store bytes to be passed to `return_value` in memory at address 0.
                    (i64.store
                        (i32.const 0)
                        (i64.const 42))

                    ;; The length is 8 since we return an `i64`. The address 0 was used above to
                    ;; store the return value.
                    (call $env.return_value
                        (i64.const 8)
                        (i64.extend_i32_u
                            (i32.const 0)))
                    )
            )
        "#,
    )
    .expect("The submodule should be valid wat")
}

#[test]
fn test_submodule_execution_with_one_resume() {
    let wasm_binary = near_test_contracts::rs_contract();
    let node = setup_runtime_node_with_contract(test_contract_account(), wasm_binary);

    let submodule_key = b"submodule1".to_vec();
    let submodule_code = submodule_yield_and_return_int();

    // Deploy submodule.
    let tx_result = node
        .user()
        .deploy_submodule(test_contract_account(), submodule_key.clone(), submodule_code.clone())
        .expect("Transaction that deploys submodule should succeed");
    assert_eq!(tx_result.status, FinalExecutionStatus::SuccessValue(Vec::new()));

    let tx_result = node
        .user()
        .function_call(
            alice_account(),
            test_contract_account(),
            "execute_submodule_with_one_resume",
            submodule_key,
            MAX_GAS,
            0,
        )
        .expect("Transaction that executes submodule should succeed");

    let mut expected_bytes = vec![];
    expected_bytes.extend(42u64.to_le_bytes()); // data the submodule should yield back
    expected_bytes.extend(43u64.to_le_bytes()); // data the submodule should return
    assert_eq!(tx_result.status, FinalExecutionStatus::SuccessValue(expected_bytes));
}

/// Returns the WebAssembly binary of a submodule that:
///
/// 1) Yields back to the main contract once passing back `42u64`.
/// 2) After resume it completes execution and returns `43u64`.
fn submodule_yield_and_return_int() -> Vec<u8> {
    wat::parse_str(
        r#"
            (module
                (type $t_env_callback (func (param i64 i64)))
                (type $t_env_return_value (func (param i64 i64)))
                (type $t_main (func))

                (import "env" "callback" (func $env.callback (type $t_env_callback)))
                (import "env" "return_value" (func $env.return_value (type $t_env_return_value)))

                (memory 1)

                (func $main (export "main") (type $t_main)
                    ;; Store bytes to be passed to `callback` in memory at address 0.
                    (i64.store
                        (i32.const 0)
                        (i64.const 42))

                    ;; Yield to the main contract. The length is 8 since we return an `i64`. The
                    ;; address 0 was used above to store the data to pass to the main contract.
                    (call $env.callback
                        (i64.const 8)
                        (i64.extend_i32_u
                            (i32.const 0)))

                    ;; Store bytes passed to `return_value` in memory at address 8 (after the bytes
                    ;; stored above).
                    (i64.store
                        (i32.const 8)
                        (i64.const 43))

                    ;; Return the bytes stored above. The length is 8 since we return an `i64`.
                    (call $env.return_value
                        (i64.const 8)
                        (i64.extend_i32_u
                            (i32.const 8)))
                    )
            )
        "#,
    )
    .expect("The submodule should be valid wat")
}
