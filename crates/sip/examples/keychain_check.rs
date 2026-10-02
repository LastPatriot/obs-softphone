// SPDX-License-Identifier: GPL-2.0-or-later
//! Round-trips a throwaway secret through the OS secret store and deletes it.
//!   cargo run -p softphone-sip --example keychain_check
use softphone_sip::{OsKeyring, SecretStore};

fn main() {
    let account = "selftest@keychain-check.invalid";
    let k = OsKeyring;
    k.set(account, "round-trip-ok").expect("set");
    let got = k.get(account).expect("get");
    k.delete(account).expect("delete");
    let after = k.get(account).expect("get after delete");
    println!("read back: {got:?}, after delete: {after:?}");
    assert_eq!(got.as_deref(), Some("round-trip-ok"));
    assert_eq!(after, None);
}
