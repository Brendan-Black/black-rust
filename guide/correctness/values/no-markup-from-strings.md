# Rust: markup, SQL, shell

`format!` is concatenation: never build HTML, SQL, or a shell line with it. Use a template engine that escapes by
default (askama, maud), bound parameters (sqlx's `query!` / `bind`), and `Command::new(program).arg(x)`, never
`Command::new("sh").arg("-c").arg(format!(...))`.
