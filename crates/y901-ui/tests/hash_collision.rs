//! 哈希碰撞统计：100 万个随机语义面，断言零碰撞。
//!
//! 目的不是"证明 FNV-1a 正确"（那是数学问题），而是确认**我们的字段拼接方式**
//! 没有把不同语义面映射到同一状态机（例如忘记喂分隔符、字段顺序错位、
//! `InputKind as u8` 截断）。
//!
//! 手动运行：
//! ```text
//! cargo test --release -p y901-ui --test hash_collision -- --ignored --nocapture
//! ```

use std::collections::HashSet;
use std::time::Instant;

use y901_ui::{hash_semantics, InputKind, Json, Role, Semantics};

/// 极简 LCG：不引入 rand crate，序列可复现。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // Numerical Recipes 参数
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 16
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[(self.next() as usize) % items.len()]
    }
}

const ROLES: &[&str] = &["button", "text_input", "text", "panel", "list", "terminal"];
const ACTIONS: &[(&str, InputKind)] = &[
    ("click", InputKind::None),
    ("set_text", InputKind::Text),
    ("scroll_to", InputKind::Number),
    ("toggle", InputKind::Bool),
    ("focus", InputKind::None),
];

fn random_semantics(rng: &mut Rng, i: usize) -> (Role, Semantics) {
    let role_name = rng.pick(ROLES);
    let role: Role = match role_name {
        "button" => Role::BUTTON,
        "text_input" => Role::TEXT_INPUT,
        "text" => Role::TEXT,
        "panel" => Role::PANEL,
        "list" => Role::LIST,
        _ => Role("terminal"),
    };

    // label 带索引 ⇒ 100 万个语义面两两不同（碰撞数 = 迭代数 - 集合大小）
    let mut sem = Semantics::default()
        .label(format!("node-{i}-{}", rng.next() % 1000))
        .value(format!("v{}", rng.next() % 1000))
        .exposed(rng.next().is_multiple_of(2))
        .sensitive(rng.next().is_multiple_of(2))
        .disabled(rng.next().is_multiple_of(2))
        .state("n", (rng.next() % 1000) as u32)
        .state("flag", rng.next().is_multiple_of(2));

    let action_count = (rng.next() % 4) as usize;
    for k in 0..action_count {
        let (name, kind) = ACTIONS[(rng.next() as usize + k) % ACTIONS.len()];
        sem = sem.action_spec(name, kind);
    }
    let _ = Json::Null; // 保证 Json 被使用（state 内部已是 Json）
    (role, sem)
}

#[test]
#[ignore = "collision spike：手动运行"]
fn one_million_semantic_surfaces_have_no_collisions() {
    const N: usize = 1_000_000;

    let mut rng = Rng(0x5eed_1234_5678_9abc);
    let mut seen: HashSet<u64> = HashSet::with_capacity(N);
    let mut hashed = 0usize;

    let t0 = Instant::now();
    for i in 0..N {
        let (role, sem) = random_semantics(&mut rng, i);
        seen.insert(hash_semantics(role, &sem));
        hashed += 1;
    }
    let elapsed = t0.elapsed();

    let collisions = hashed - seen.len();
    eprintln!(
        "[collision] hashed={hashed} distinct={} collisions={collisions} elapsed={elapsed:?} ({:.2} us/hash)",
        seen.len(),
        elapsed.as_secs_f64() * 1e6 / hashed as f64
    );

    assert_eq!(
        collisions, 0,
        "不同语义面产生了相同哈希（{collisions} 个）——检查字段分隔或截断"
    );
}

#[test]
#[ignore = "collision spike：手动运行"]
fn field_boundary_is_not_ambiguous() {
    // 不加分隔符时 "ab"+"c" 与 "a"+"bc" 会撞车；这条测试确认分隔符真的生效
    let a = Semantics::default().label("ab").value("c");
    let b = Semantics::default().label("a").value("bc");
    assert_ne!(
        hash_semantics(Role::BUTTON, &a),
        hash_semantics(Role::BUTTON, &b),
        "字段边界必须有分隔，否则不同切分会碰撞"
    );
}
