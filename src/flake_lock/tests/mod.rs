// SPDX-FileCopyrightText: 2021 Serokell <https://serokell.io>
//
// SPDX-License-Identifier: MPL-2.0

use super::*;

// If you add a new valid flake.lock to resources, also add it here
const ALL_RESOURCES: &[&str] = &["simple_old", "simple_new"];

use std::path::PathBuf;

fn get_resources(test: &'static str) -> PathBuf {
    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    d.push(format!("src/flake_lock/tests/{}", test));
    d
}

#[test]
fn parses_locks_correctly() {
    for res in ALL_RESOURCES {
        let mut repo = get_resources(res);

        let lock = get_lock(repo.as_path()).unwrap();

        repo.push("lock.expected");

        let lock_golden = std::fs::read_to_string(repo).unwrap();

        assert_eq!(format!("{:#?}", lock), lock_golden);
    }
}

#[test]
fn diffs_correct() {
    for res1 in ALL_RESOURCES {
        let repo1 = get_resources(res1);

        let lock1 = get_lock(repo1.as_path()).unwrap();

        for res2 in ALL_RESOURCES {
            let repo2 = get_resources(res2);

            let lock2 = get_lock(repo2.as_path()).unwrap();

            eprintln!(
                "{}.diff({})\n{:#?}",
                repo1.to_string_lossy(),
                repo2.to_string_lossy(),
                lock1.diff(&lock2).unwrap()
            );

            let mut expected_path = get_resources(res1);
            expected_path.push(format!("{}.expected", res2));

            let expected = std::fs::read_to_string(expected_path).unwrap();

            assert_eq!(format!("{:#?}", lock1.diff(&lock2).unwrap()), expected);
        }
    }
}

#[test]
fn link_github() {
    let repo1 = get_resources("simple_old");

    let lock1 = get_lock(repo1.as_path()).unwrap();

    let repo2 = get_resources("simple_new");

    let lock2 = get_lock(repo2.as_path()).unwrap();

    let link = lock1
        .diff(&lock2)
        .unwrap()
        .0
        .get("nixpkgs")
        .unwrap()
        .link()
        .unwrap();

    let expected = "https://github.com/NixOS/nixpkgs/compare/84d74ae9c9cbed73274b8e4e00be14688ffc93fe...c601d56e19dd2ed71b23d8aa76be8437d043d4c5?expand=1".to_string();

    assert_eq!(link, expected);
}
