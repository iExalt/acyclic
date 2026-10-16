use hegel::TestCase;
use hegel::generators as gs;

use super::*;

#[test]
fn numbers_sort_by_value_and_the_rest_as_text() {
    let mut titles =
        vec!["Step 10", "Step 2", "step 1", "Step 02", "Step", "10", "9", "Step 2b", "Step 2a"];
    titles.sort_by(|a, b| cmp(a, b));
    assert_eq!(
        titles,
        ["9", "10", "Step", "step 1", "Step 02", "Step 2", "Step 2a", "Step 2b", "Step 10"]
    );
}

/// Natural order is a total order: antisymmetric, equal only for equal strings, and transitive.
#[hegel::test]
fn natural_order_is_total(tc: TestCase) {
    let text = || tc.draw(gs::from_regex("[abAB0-29 ]{0,6}"));
    let (a, b, c) = (text(), text(), text());
    assert_eq!(cmp(&a, &b), cmp(&b, &a).reverse());
    assert_eq!(cmp(&a, &b) == Ordering::Equal, a == b);
    if cmp(&a, &b).is_le() && cmp(&b, &c).is_le() {
        assert!(cmp(&a, &c).is_le(), "{a:?} {b:?} {c:?}");
    }
}
