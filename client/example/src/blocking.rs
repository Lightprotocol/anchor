use anchor_client::solana_sdk::pubkey::Pubkey;
use anchor_client::solana_sdk::signature::{Keypair, Signer};
use anchor_client::solana_sdk::system_instruction;
use anchor_client::{Client, Cluster};
use anyhow::Result;
use clap::Parser;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::signature::read_keypair_file;
use solana_sdk::system_program;
// The `accounts` and `instructions` modules are generated by the framework.
use basic_2::accounts as basic_2_accounts;
use basic_2::instruction as basic_2_instruction;
use basic_2::Counter;
use events::instruction as events_instruction;
use events::MyEvent;
use optional::accounts::Initialize as OptionalInitialize;
use optional::instruction as optional_instruction;
// The `accounts` and `instructions` modules are generated by the framework.
use basic_4::accounts as basic_4_accounts;
use basic_4::instruction as basic_4_instruction;
use basic_4::Counter as CounterAccount;
// The `accounts` and `instructions` modules are generated by the framework.
use crate::Opts;
use composite::accounts::{Bar, CompositeUpdate, Foo, Initialize};
use composite::instruction as composite_instruction;
use composite::{DummyA, DummyB};
use optional::account::{DataAccount, DataPda};
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

type TestFn<C> = &'static (dyn Fn(&Client<C>, Pubkey) -> Result<()> + Send + Sync);

pub fn main() -> Result<()> {
    let opts = Opts::parse();

    // Wallet and cluster params.
    let payer = read_keypair_file(&*shellexpand::tilde("~/.config/solana/id.json"))
        .expect("Example requires a keypair file");
    let url = Cluster::Custom(
        "http://localhost:8899".to_string(),
        "ws://127.0.0.1:8900".to_string(),
    );

    if !opts.multithreaded {
        // Client.
        let payer = Rc::new(payer);
        let client =
            Client::new_with_options(url.clone(), payer.clone(), CommitmentConfig::processed());

        // Run tests on single thread with a single client using an Rc
        println!("\nStarting single thread test...");
        composite(&client, opts.composite_pid)?;
        basic_2(&client, opts.basic_2_pid)?;
        basic_4(&client, opts.basic_4_pid)?;

        // Can also use references, since they deref to a signer
        let payer: &Keypair = &payer;
        let client = Client::new_with_options(url, payer, CommitmentConfig::processed());
        events(&client, opts.events_pid)?;
        optional(&client, opts.optional_pid)?;
    } else {
        // Client.
        let payer = Arc::new(payer);
        let client = Client::new_with_options(url, payer, CommitmentConfig::processed());
        let client = Arc::new(client);

        // Run tests multithreaded while sharing a client
        println!("\nStarting multithread test...");
        let client = Arc::new(client);
        let tests: Vec<(TestFn<Arc<Keypair>>, Pubkey)> = vec![
            (&composite, opts.composite_pid),
            (&basic_2, opts.basic_2_pid),
            (&basic_4, opts.basic_4_pid),
            (&events, opts.events_pid),
            (&optional, opts.optional_pid),
        ];
        let mut handles = vec![];
        for (test, arg) in tests {
            let local_client = Arc::clone(&client);
            handles.push(std::thread::spawn(move || test(&local_client, arg)));
        }
        for handle in handles {
            assert!(handle.join().unwrap().is_ok());
        }
    }

    // Success.
    Ok(())
}

// Runs a client for examples/tutorial/composite.
//
// Make sure to run a localnet with the program deploy to run this example.
pub fn composite<C: Deref<Target = impl Signer> + Clone>(
    client: &Client<C>,
    pid: Pubkey,
) -> Result<()> {
    // Program client.
    let program = client.program(pid)?;

    // `Initialize` parameters.
    let dummy_a = Keypair::new();
    let dummy_b = Keypair::new();

    // Build and send a transaction.
    program
        .request()
        .instruction(system_instruction::create_account(
            &program.payer(),
            &dummy_a.pubkey(),
            program.rpc().get_minimum_balance_for_rent_exemption(500)?,
            500,
            &program.id(),
        ))
        .instruction(system_instruction::create_account(
            &program.payer(),
            &dummy_b.pubkey(),
            program.rpc().get_minimum_balance_for_rent_exemption(500)?,
            500,
            &program.id(),
        ))
        .signer(&dummy_a)
        .signer(&dummy_b)
        .accounts(Initialize {
            dummy_a: dummy_a.pubkey(),
            dummy_b: dummy_b.pubkey(),
        })
        .args(composite_instruction::Initialize)
        .send()?;

    // Assert the transaction worked.
    let dummy_a_account: DummyA = program.account(dummy_a.pubkey())?;
    let dummy_b_account: DummyB = program.account(dummy_b.pubkey())?;
    assert_eq!(dummy_a_account.data, 0);
    assert_eq!(dummy_b_account.data, 0);

    // Build and send another transaction, using composite account parameters.
    program
        .request()
        .accounts(CompositeUpdate {
            foo: Foo {
                dummy_a: dummy_a.pubkey(),
            },
            bar: Bar {
                dummy_b: dummy_b.pubkey(),
            },
        })
        .args(composite_instruction::CompositeUpdate {
            dummy_a: 1234,
            dummy_b: 4321,
        })
        .send()?;

    // Assert the transaction worked.
    let dummy_a_account: DummyA = program.account(dummy_a.pubkey())?;
    let dummy_b_account: DummyB = program.account(dummy_b.pubkey())?;
    assert_eq!(dummy_a_account.data, 1234);
    assert_eq!(dummy_b_account.data, 4321);

    println!("Composite success!");

    Ok(())
}

// Runs a client for examples/tutorial/basic-2.
//
// Make sure to run a localnet with the program deploy to run this example.
pub fn basic_2<C: Deref<Target = impl Signer> + Clone>(
    client: &Client<C>,
    pid: Pubkey,
) -> Result<()> {
    let program = client.program(pid)?;

    // `Create` parameters.
    let counter = Keypair::new();
    let authority = program.payer();

    // Build and send a transaction.
    program
        .request()
        .signer(&counter)
        .accounts(basic_2_accounts::Create {
            counter: counter.pubkey(),
            user: authority,
            system_program: system_program::ID,
        })
        .args(basic_2_instruction::Create { authority })
        .send()?;

    let counter_account: Counter = program.account(counter.pubkey())?;

    assert_eq!(counter_account.authority, authority);
    assert_eq!(counter_account.count, 0);

    println!("Basic 2 success!");

    Ok(())
}

pub fn events<C: Deref<Target = impl Signer> + Clone>(
    client: &Client<C>,
    pid: Pubkey,
) -> Result<()> {
    let program = client.program(pid)?;

    let (sender, receiver) = std::sync::mpsc::channel();
    let event_unsubscriber = program.on(move |_, event: MyEvent| {
        if sender.send(event).is_err() {
            println!("Error while transferring the event.")
        }
    })?;

    sleep(Duration::from_millis(1000));

    program
        .request()
        .args(events_instruction::Initialize {})
        .send()?;

    let event = receiver.recv().unwrap();
    assert_eq!(event.data, 5);
    assert_eq!(event.label, "hello".to_string());

    event_unsubscriber.unsubscribe();

    println!("Events success!");

    Ok(())
}

pub fn basic_4<C: Deref<Target = impl Signer> + Clone>(
    client: &Client<C>,
    pid: Pubkey,
) -> Result<()> {
    let program = client.program(pid)?;
    let authority = program.payer();
    let (counter, _) = Pubkey::find_program_address(&[b"counter"], &pid);

    program
        .request()
        .accounts(basic_4_accounts::Initialize {
            counter,
            authority,
            system_program: system_program::ID,
        })
        .args(basic_4_instruction::Initialize {})
        .send()?;
    let counter_account: CounterAccount = program.account(counter)?;
    assert_eq!(counter_account.authority, authority);
    assert_eq!(counter_account.count, 0);

    program
        .request()
        .accounts(basic_4_accounts::Increment { counter, authority })
        .args(basic_4_instruction::Increment {})
        .send()?;

    let counter_account: CounterAccount = program.account(counter)?;
    assert_eq!(counter_account.authority, authority);
    assert_eq!(counter_account.count, 1);

    println!("Basic 4 success!");

    Ok(())
}

// Runs a client for tests/optional.
//
// Make sure to run a localnet with the program deploy to run this example.
pub fn optional<C: Deref<Target = impl Signer> + Clone>(
    client: &Client<C>,
    pid: Pubkey,
) -> Result<()> {
    // Program client.
    let program = client.program(pid)?;

    // `Initialize` parameters.
    let data_account_keypair = Keypair::new();

    let data_account_key = data_account_keypair.pubkey();

    let data_pda_seeds = &[DataPda::PREFIX.as_ref(), data_account_key.as_ref()];
    let data_pda_key = Pubkey::find_program_address(data_pda_seeds, &pid).0;
    let required_keypair = Keypair::new();
    let value: u64 = 10;

    // Build and send a transaction.

    program
        .request()
        .instruction(system_instruction::create_account(
            &program.payer(),
            &required_keypair.pubkey(),
            program
                .rpc()
                .get_minimum_balance_for_rent_exemption(DataAccount::LEN)?,
            DataAccount::LEN as u64,
            &program.id(),
        ))
        .signer(&data_account_keypair)
        .signer(&required_keypair)
        .accounts(OptionalInitialize {
            payer: Some(program.payer()),
            required: required_keypair.pubkey(),
            system_program: Some(system_program::id()),
            optional_account: Some(data_account_keypair.pubkey()),
            optional_pda: None,
        })
        .args(optional_instruction::Initialize { value, key: pid })
        .send()
        .unwrap();

    // Assert the transaction worked.
    let required: DataAccount = program.account(required_keypair.pubkey())?;
    assert_eq!(required.data, 0);

    let optional_pda = program.account::<DataPda>(data_pda_key);
    assert!(optional_pda.is_err());

    let optional_account: DataAccount = program.account(data_account_keypair.pubkey())?;
    assert_eq!(optional_account.data, value * 2);

    println!("Optional success!");

    Ok(())
}
