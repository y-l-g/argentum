# Storefront: Argentum

The Argentum (Topcoat-based) app under the benchmark harness. It declares the
`Author`, `Post`, and `Comment` models and the `Author` and `Post` resources,
seeds 5 authors, 50 posts, and 50 comments into in-memory SQLite, and either
serves the panel or runs the in-process bench:

```sh
# Serve the panel at http://localhost:3000/ (try /admin/posts)
cargo run

# Measure the list path in process; see ../README.md for the workload.
cargo run -- --bench --iterations 100
```
