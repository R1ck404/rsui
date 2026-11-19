//! Sampling where JavaScript is, as the inspector's Record does: a function that's busy for a while is found by name, in
//! the function that called it.

#[cfg(windows)]
#[test]
fn a_busy_function_is_sampled_by_name() {
    let mut engine = rsui_js::Engine::default();
    engine
        .eval(
            "function spin(ms) { const end = Date.now() + ms; let n = 0; while (Date.now() < end) { n += 1; } return n; }\n\
             function outer() { return spin(120); }",
            "busy.js",
        )
        .unwrap();
    let (_, from) = rsui_spans::since(u64::MAX);
    engine.start_sampling(std::time::Duration::from_millis(1));
    assert!(engine.sampling());
    engine.eval("outer()", "run.js").unwrap();
    engine.stop_sampling();
    assert!(!engine.sampling());
    let (recs, _) = rsui_spans::since(from);
    let samples: Vec<_> = recs
        .iter()
        .filter(|rec| rec.kind == rsui_spans::Kind::Sample as u8)
        .collect();
    assert!(samples.len() >= 20, "120 ms at about one a millisecond gives plenty: {}", samples.len());
    let stacks = rsui_spans::stacks_from(0);
    let named = |label: u32| rsui_spans::label(label);
    let in_spin = samples
        .iter()
        .filter(|sample| {
            let stack = &stacks[sample.label as usize];
            let names: Vec<String> = stack.iter().map(|label| named(*label)).collect();
            names.last().is_some_and(|inner| inner.starts_with("spin\u{1}"))
                && names.iter().any(|name| name.starts_with("outer\u{1}"))
        })
        .count();
    assert!(
        in_spin * 2 >= samples.len(),
        "most samples are in spin, called from outer: {in_spin} of {}, e.g. {:?}",
        samples.len(),
        stacks[samples[0].label as usize].iter().map(|label| named(*label)).collect::<Vec<_>>()
    );
    let place = named(stacks[samples[samples.len() / 2].label as usize].last().copied().unwrap());
    assert!(place.contains("busy.js:"), "it says where: {place}");
}
