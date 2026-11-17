//! The invoices example, used as it would be, with the steps an agent gives `--do`. Each has a file of its own to save to.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn place(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("invoices-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder.join("invoices.json")
}

/// Does these, and says what it printed.
fn steps(file: &PathBuf, steps: &str) -> String {
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/apps/invoices.tsx");
    let output = Command::new(TOOL)
        .env("RSUI_INVOICES_FILE", file)
        .arg("--do")
        .arg(steps)
        .arg(app)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert!(
        output.status.success(),
        "{steps}\n{said}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    said
}

#[test]
fn it_opens_on_what_is_at_the_top_of_the_list() {
    // What's on that invoice is only there if that's the one that's open.
    steps(
        &place("top"),
        "expect \"Workshop\"; expect no \"Design work\"",
    );
}

#[test]
fn what_is_shown_always_adds_up() {
    // Tax on this comes to 787.815, which rounded on its own, and in the total, would be a cent apart.
    steps(
        &place("adds-up"),
        "click #new; type #client Acme; type #quantity-1 3; type #price-1 1250.50;
         expect \"Subtotal €3,751.50\"; expect \"Tax at 21% €787.82\"; expect \"Total €4,539.32\"",
    );
}

#[test]
fn a_comma_works_in_a_price_and_what_is_not_a_number_says_so() {
    steps(
        &place("numbers"),
        "click #new; type #price-1 19,99; expect \"€19.99\"; type #quantity-1 lots; expect \"Not a number\"; expect \"Subtotal €0.00\"",
    );
}

#[test]
fn what_is_wrong_with_an_invoice_is_said() {
    steps(
        &place("problems"),
        "click #new; expect \"An invoice needs a client\"; type #client Acme; expect no \"needs a client\";
         type #due soon; expect \"Use the form\"; type #due 2020-01-01; expect \"due before it was issued\"",
    );
}

#[test]
fn changing_the_tax_in_the_settings_only_changes_new_invoices() {
    let file = place("tax");
    steps(
        &file,
        "click #invoice-1; expect \"Total €1,282.60\";
         click #settings; window \"Settings\"; press right #tax; window 0; expect \"Total €1,282.60\"; expect \"Tax at 21%\";
         click #invoice-4; click #use-tax; expect \"Tax at 22%\";
         click #new; expect \"Tax at 22%\"; close",
    );
}

#[test]
fn searching_and_filtering_show_what_they_should() {
    steps(
        &place("filters"),
        "type #search contoso; expect \"INV-1003 Contoso\"; expect no \"INV-1001\"; type #search nobody; expect \"Nothing matches that\";
         type #search \"\"; click #filter-3; expect \"INV-1002 Blue\"; expect no \"INV-1003\"; click #filter-4; expect \"INV-1001 North\"",
    );
}

#[test]
fn paying_something_overdue_moves_the_money_in_the_summary() {
    steps(
        &place("pay"),
        "expect \"€1,161.60\"; click #invoice-2; click #pay; expect \"€0.00\"; expect \"€2,444.20\"",
    );
}

#[test]
fn deleting_asks_first_and_nothing_else_can_be_done_meanwhile() {
    steps(
        &place("delete"),
        "click #invoice-2; click #delete; window \"Delete it?\"; click #keep; window 0; expect \"INV-1002 Blue\";
         click #delete; window \"Delete it?\"; click #yes-delete; window 0; expect no \"INV-1002\"",
    );
}

#[test]
fn it_exports_where_it_is_told_to() {
    let file = place("export");
    let out = file.with_file_name("out.csv");
    steps(
        &file,
        &format!(
            "click #invoice-3; dialog \"{}\"; click #export; wait \"Exported to\"",
            out.display().to_string().replace('\\', "\\\\")
        ),
    );
    let csv = std::fs::read_to_string(out).unwrap();
    assert!(csv.starts_with("What,Quantity,Price,Total\n"), "{csv}");
    assert!(csv.contains("Development,30,95,2850.00\n"), "{csv}");
    assert!(csv.contains("Tax at 21%,,,648.90\n"), "{csv}");
    assert!(csv.ends_with("Total,,,3738.90\n"), "{csv}");
}

#[test]
fn closing_straight_after_a_change_keeps_it() {
    let file = place("keeps");
    steps(&file, "click #new; type #client \"Quick One\"; close");
    steps(&file, "expect \"Quick One\"");
}

#[test]
fn what_was_saved_before_invoices_had_their_own_tax_still_opens() {
    let file = place("old");
    std::fs::write(
        &file,
        r#"{"invoices":[{"id":1,"number":"INV-1","client":"Old","issued":"2026-01-01","due":"2026-02-01","status":"paid",
            "lines":[{"id":1,"what":"Thing","quantity":"1","price":"100"}]}],"settings":{"tax":10,"currency":0},"nextId":2}"#,
    )
    .unwrap();
    steps(&file, "expect \"Tax at 10%\"; expect \"Total €110.00\"");
}
