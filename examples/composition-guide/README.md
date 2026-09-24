# 七種組合用法

這個 package 以可執行的 `#[bundle]`、`#[application]`、`#[provide]` 與 `#[hook(start)]` 範例展示 framework 的組合方式。共用宣告與支援程式碼見 [shared library](src/lib.rs)；每個案例都可單獨執行：

```sh
cargo run -p composition-guide --bin <name>
```

`#[hook(start)]` 是明確的 graph root：匯入 bundle 後，其中宣告的 hook 會執行，hook 依賴的 providers 也會變成 reachable。`#[provide(lifecycle)]` 與 `register(...)` 是 reachable provider 的 capability；只有實作 trait 不會建立 root。

## 案例

### [`unimported_bundle`](src/bin/unimported_bundle.rs)

執行 `cargo run -p composition-guide --bin unimported_bundle`。程式沒有 import library 已宣告的 base bundle，只留下 app 自己的 hook：

```text
hook.unimported.app
```

### [`unused_import`](src/bin/unused_import.rs)

執行 `cargo run -p composition-guide --bin unused_import`。Metrics bundle 已匯入，但沒有 hook 或依賴選用 Metrics，因此沒有 Metrics 建構、啟動或關閉事件：

```text
hook.unused-import.app
```

### [`explicit_hook`](src/bin/explicit_hook.rs)

執行 `cargo run -p composition-guide --bin explicit_hook`。base bundle 的 explicit hook 即使不是 app hook 的直接依賴仍會執行；它所需的 Config provider 會先建構：

```text
construct.config
hook.base
hook.explicit.app
```

### [`transitive_lifecycle`](src/bin/transitive_lifecycle.rs)

執行 `cargo run -p composition-guide --bin transitive_lifecycle`。app hook 注入 Server；Server 透過 backend 間接依賴另一 bundle 的 Database，且同時展示 `Arc<T>` 與 `&T` 依賴。啟動依賴先後，關閉則反序：

```text
construct.config
construct.database
construct.server-backend
construct.server
start.lifecycle.database
start.lifecycle.server
hook.base
hook.transitive.server
shutdown.lifecycle.server
shutdown.lifecycle.database
```

### [`registration_lifecycle`](src/bin/registration_lifecycle.rs)

執行 `cargo run -p composition-guide --bin registration_lifecycle`。base 與 app route providers 都可達；兩者依組合順序註冊同一個 `home` key，app route 成為有效值。hook 在結尾發出完成通知，外部執行緒收到後發 stop，之後反序關閉：

```text
register.route.common
register.route.application
start.lifecycle.base-routes
start.lifecycle.app-routes
hook.registration.home=application
stop.signal
shutdown.lifecycle.app-routes
shutdown.lifecycle.base-routes
```

### [`partial_bundle`](src/bin/partial_bundle.rs)

執行 `cargo run -p composition-guide --bin partial_bundle`。hook 以 `#[inject("primary")]` 選用 primary service；同 bundle 的 alternate service 與 worker 不可達，不會建構或執行 Lifecycle：

```text
construct.config
construct.service.primary
start.lifecycle.service.primary
hook.base
hook.partial.service=primary
shutdown.lifecycle.service.primary
```

### [`startup_failure`](src/bin/startup_failure.rs)

執行 `cargo run -p composition-guide --bin startup_failure`。所有 reachable providers 在 Lifecycle startup 前建構；第二個 component start 失敗後，不啟動後續 component 或 hook。先前成功啟動的 component 仍會 shutdown；其錯誤與 start 錯誤一併出現在 `RunError.failures()`：

```text
expected RunError: 2 lifecycle failure(s); 1: ... start failed: startup failed; 2: ... shutdown failed: ready cleanup failed
failure: phase=start, component=...StartupFailure..., source=startup failed
failure: phase=shutdown, component=...StartupReady..., source=ready cleanup failed
construct.startup.ready
construct.startup.failure
construct.startup.never-started
start.lifecycle.ready
start.lifecycle.failure
shutdown.lifecycle.ready
```

`StartupNeverStarted` 已建構，但 start 與 shutdown 都不會執行；`hook.startup-failure.unexpected` 也不會出現。

正常案例輸出事件 trace；`startup_failure` 則把預期 lifecycle error 當作示範結果印出。所有元件只模擬設定、註冊及生命週期，不啟動網路服務或連接外部系統。
