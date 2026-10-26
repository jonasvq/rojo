use std::{
    cell::RefCell,
    marker::PhantomData,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Once,
};

struct Operation {
    action: &'static str,
    path: PathBuf,
}

thread_local! {
    static OPERATIONS: RefCell<Vec<Operation>> = const { RefCell::new(Vec::new()) };
}

pub(crate) struct OperationGuard {
    depth: usize,
    _thread_local: PhantomData<Rc<()>>,
}

pub(crate) fn enter(action: &'static str, path: &Path) -> OperationGuard {
    let depth = OPERATIONS.with(|operations| {
        let mut operations = operations.borrow_mut();
        let depth = operations.len();
        operations.push(Operation {
            action,
            path: path.to_owned(),
        });
        depth
    });
    OperationGuard {
        depth,
        _thread_local: PhantomData,
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let _ = OPERATIONS.try_with(|operations| {
            if let Ok(mut operations) = operations.try_borrow_mut() {
                operations.truncate(self.depth);
            }
        });
    }
}

pub(crate) fn install() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // The panic hook must not acquire tree/VFS locks: either may already
            // be held by the operation that panicked.
            let _ = OPERATIONS.try_with(|operations| {
                if let Ok(operations) = operations.try_borrow() {
                    for operation in operations.iter().rev() {
                        log::error!("While {}: {}", operation.action, operation.path.display());
                    }
                }
            });
            previous(info);
        }));
    });
}

#[cfg(test)]
mod test {
    use super::{enter, install, OPERATIONS};
    use std::path::Path;

    #[test]
    fn panic_report_identifies_the_active_filesystem_path() {
        const CHILD: &str = "ROJO_PANIC_CONTEXT_TEST_CHILD";
        const PATH: &str = r"C:\RojoRepro\missing\AgentScript.luau";
        if std::env::var_os(CHILD).is_some() {
            let _ = env_logger::try_init();
            install();
            let _operation = enter("processing filesystem event", Path::new(PATH));
            panic!("intentional diagnostic test panic");
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "diagnostic_context::test::panic_report_identifies_the_active_filesystem_path",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("RUST_LOG", "error")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(PATH),
            "panic report omitted active path: {stderr}"
        );
        assert!(stderr.contains("processing filesystem event"), "{stderr}");
    }

    #[test]
    fn nested_context_is_restored_without_leaking_into_the_next_event() {
        {
            let _outer = enter("outer", Path::new("first.luau"));
            {
                let _inner = enter("inner", Path::new("second.luau"));
                OPERATIONS.with(|operations| assert_eq!(operations.borrow().len(), 2));
            }
            OPERATIONS.with(|operations| {
                assert_eq!(operations.borrow()[0].path, Path::new("first.luau"))
            });
        }
        OPERATIONS.with(|operations| assert!(operations.borrow().is_empty()));
    }
}
