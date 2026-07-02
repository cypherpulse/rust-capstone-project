#![allow(unused)]
use bitcoin::hex::DisplayHex;
use bitcoincore_rpc::bitcoin::Amount;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use serde::Deserialize;
use serde_json::json;
use std::fs::File;
use std::io::Write;

// Node access params
const RPC_URL: &str = "http://127.0.0.1:18443"; // Default regtest RPC port
const RPC_USER: &str = "alice";
const RPC_PASS: &str = "password";
const MINER_WALLET: &str = "Miner";
const TRADER_WALLET: &str = "Trader";
const MINER_REWARD_LABEL: &str = "Mining Reward";
const TRADER_RECEIVE_LABEL: &str = "Received";
const SEND_AMOUNT_BTC: f64 = 20.0;

// You can use calls not provided in RPC lib API using the generic `call` function.
// An example of using the `send` RPC call, which doesn't have exposed API.
// You can also use serde_json `Deserialize` derivation to capture the returned json result.
fn send(rpc: &Client, addr: &str) -> bitcoincore_rpc::Result<String> {
    let args = [
        json!([{addr : 100 }]), // recipient address
        json!(null),            // conf target
        json!(null),            // estimate mode
        json!(null),            // fee rate in sats/vb
        json!(null),            // Empty option object
    ];

    #[derive(Deserialize)]
    struct SendResult {
        complete: bool,
        txid: String,
    }
    let send_result = rpc.call::<SendResult>("send", &args)?;
    assert!(send_result.complete);
    Ok(send_result.txid)
}

fn ensure_wallet(rpc: &Client, wallet_name: &str) -> bitcoincore_rpc::Result<()> {
    let loaded_wallets: Vec<String> = rpc.call("listwallets", &[])?;
    if loaded_wallets.iter().any(|name| name == wallet_name) {
        return Ok(());
    }

    let wallet_dir: serde_json::Value = rpc.call("listwalletdir", &[])?;
    let wallet_exists = wallet_dir["wallets"]
        .as_array()
        .map(|wallets| {
            wallets.iter().any(|wallet| {
                wallet
                    .get("name")
                    .and_then(|name| name.as_str())
                    .map(|name| name == wallet_name)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    if wallet_exists {
        rpc.call::<serde_json::Value>("loadwallet", &[json!(wallet_name)])?;
        return Ok(());
    }

    rpc.call::<serde_json::Value>("createwallet", &[json!(wallet_name)])?;
    Ok(())
}

fn wallet_client(wallet_name: &str) -> bitcoincore_rpc::Result<Client> {
    let url = format!("{RPC_URL}/wallet/{wallet_name}");
    Client::new(&url, Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()))
}

fn write_output(path: &str, values: &[String]) -> std::io::Result<()> {
    let mut file = File::create(path)?;
    for value in values {
        writeln!(file, "{}", value)?;
    }
    Ok(())
}

fn main() -> bitcoincore_rpc::Result<()> {
    let rpc = Client::new(
        RPC_URL,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    let blockchain_info = rpc.get_blockchain_info()?;
    println!("Blockchain Info: {:?}", blockchain_info);

    // Create/Load the wallets, named 'Miner' and 'Trader'. Have logic to optionally create/load them if they do not exist or not loaded already.
    ensure_wallet(&rpc, MINER_WALLET)?;
    ensure_wallet(&rpc, TRADER_WALLET)?;

    let miner_rpc = wallet_client(MINER_WALLET)?;
    let trader_rpc = wallet_client(TRADER_WALLET)?;

    // Generate spendable balances in the Miner wallet. How many blocks needs to be mined?
    let miner_reward_address: String = miner_rpc.call("getnewaddress", &[json!(MINER_REWARD_LABEL)])?;
    println!("Miner reward address: {miner_reward_address}");

    let mut mined_blocks = 0;
    loop {
        let miner_balance: f64 = miner_rpc.call("getbalance", &[])?;
        if miner_balance > 0.0 {
            break;
        }

        miner_rpc.call::<Vec<String>>(
            "generatetoaddress",
            &[json!(1), json!(miner_reward_address.clone()), json!(null)],
        )?;
        mined_blocks += 1;
    }

    println!("Note: Coinbase rewards are immature until 100 confirmations; wallet balance remains zero until maturity.");
    let miner_balance: f64 = miner_rpc.call("getbalance", &[])?;
    println!("Miner balance: {}", miner_balance);

    // Load Trader wallet and generate a new address
    let trader_receive_address: String = trader_rpc.call("getnewaddress", &[json!(TRADER_RECEIVE_LABEL)])?;

    // Send 20 BTC from Miner to Trader
    let txid: String = miner_rpc.call(
        "sendtoaddress",
        &[
            json!(trader_receive_address.clone()),
            json!(SEND_AMOUNT_BTC),
            json!(null),
            json!(null),
        ],
    )?;
    println!("Transaction sent with txid: {txid}");

    // Check transaction in mempool
    let mempool_entry: serde_json::Value = miner_rpc.call("getmempoolentry", &[json!(txid.clone())])?;
    println!("Mempool entry: {:?}", mempool_entry);

    // Mine 1 block to confirm the transaction
    let confirmed_blocks: Vec<String> = miner_rpc.call(
        "generatetoaddress",
        &[json!(1), json!(miner_reward_address.clone()), json!(null)],
    )?;
    println!("Confirmed in block: {}", confirmed_blocks[0]);

    // Extract all required transaction details
    let tx_info: serde_json::Value = miner_rpc.call("gettransaction", &[json!(txid.clone()), json!(null), json!(true)])?;

    let miner_input_address = tx_info["decoded"]["vin"][0]["prevout"]["scriptPubKey"]["addresses"]
        .get(0)
        .and_then(|v| v.as_str())
        .unwrap_or(&miner_reward_address)
        .to_string();

    let miner_input_amount = tx_info["decoded"]["vin"][0]["prevout"]["value"]
        .as_f64()
        .unwrap_or(0.0);

    let mut trader_output_address = String::new();
    let mut trader_output_amount = 0.0;
    if let Some(vouts) = tx_info["decoded"]["vout"].as_array() {
        for vout in vouts {
            if let Some(addresses) = vout["scriptPubKey"]["addresses"].as_array() {
                if addresses.iter().any(|address| address.as_str() == Some(trader_receive_address.as_str())) {
                    trader_output_address = trader_receive_address.clone();
                    trader_output_amount = vout["value"].as_f64().unwrap_or(0.0);
                    break;
                }
            }
        }
    }

    let mut change_output = String::new();
    let mut change_amount = 0.0;
    if let Some(vouts) = tx_info["decoded"]["vout"].as_array() {
        for vout in vouts {
            if let Some(addresses) = vout["scriptPubKey"]["addresses"].as_array() {
                if let Some(address) = addresses.iter().find_map(|address| address.as_str()) {
                    if address != trader_output_address && address != miner_input_address {
                        change_output = address.to_string();
                        change_amount = vout["value"].as_f64().unwrap_or(0.0);
                        break;
                    }
                }
            }
        }
    }
    if change_output.is_empty() {
        change_output = miner_reward_address.clone();
    }

    let fee = tx_info["fee"].as_f64().unwrap_or(0.0);
    let block_height = tx_info["blockheight"].as_i64().unwrap_or(0);
    let block_hash = tx_info["blockhash"].as_str().unwrap_or("").to_string();

    let output_lines = vec![
        txid.clone(),
        miner_input_address,
        miner_input_amount.to_string(),
        trader_output_address,
        trader_output_amount.to_string(),
        change_output,
        change_amount.to_string(),
        fee.to_string(),
        block_height.to_string(),
        block_hash,
    ];

    // Write the data to ../out.txt in the specified format given in readme.md
    write_output("../out.txt", &output_lines)?;
    println!("Wrote transaction details to ../out.txt");

    Ok(())
}
