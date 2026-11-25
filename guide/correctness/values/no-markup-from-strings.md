# Rust: markup, SQL, shell

`format!` is concatenation: don't build HTML, SQL, or a shell line with it. Use a template engine that escapes by
default (askama, maud), bound parameters (sqlx's `query!` / `bind`), and `Command::new(program).arg(x)`, not
`Command::new("sh").arg("-c").arg(format!(...))`.
