//! 全局单例槽：生产期进程级，测试期线程级。
//!
//! 同一套「全局单例 + 测试期隔离 + RAII 清理」此前在 `paths` 与
//! `plugin::manifest` 各写一遍：各自一个 `OnceLock`、一个 `thread_local!`、
//! 各自在 `init` / `get` 里做 cfg 分支、各自一个 `Drop` 清理 guard。
//! 这里收成一份：使用方只声明「我有一个全局单例」，隔离与清理由本模块负责。
//!
//! 生产期为什么是进程级、测试期为什么是线程级：`OnceLock` 进程内只写一次，
//! 而测试要能并行跑、各自持有一份互不干扰的主目录/平台目录。

#[cfg(not(test))]
use std::sync::OnceLock;

#[cfg(test)]
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::HashMap,
    marker::PhantomData,
};

// 测试期每线程一份的槽：按类型索引。按类型索引而非「每个类型一个
// `thread_local!`」：后者要求为每个类型声明一个静态，泛型化不了；前者让本
// 模块成为唯一的样板来源。
#[cfg(test)]
thread_local! {
    static TEST_SLOTS: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

/// 全局单例槽。
///
/// 生产期：进程内一份（`OnceLock`，重复写入以首次为准）。测试期：每线程
/// 一份（线程局部表）——测试要能并行跑，各自持有一份互不干扰。
pub(crate) struct Global<T> {
    #[cfg(not(test))]
    production: OnceLock<T>,
    #[cfg(test)]
    _marker: PhantomData<T>,
}

impl<T: Clone + 'static> Global<T> {
    /// 声明一个全局单例槽（可作 `static` 用）。
    pub(crate) const fn new() -> Self {
        Self {
            #[cfg(not(test))]
            production: OnceLock::new(),
            #[cfg(test)]
            _marker: PhantomData,
        }
    }

    /// 写入全局值：生产期以首次为准，测试期每个线程各自一份。
    pub(crate) fn set(&self, value: T) {
        #[cfg(not(test))]
        {
            let _ = self.production.set(value);
        }
        #[cfg(test)]
        {
            TEST_SLOTS.with(|slots| {
                slots
                    .borrow_mut()
                    .insert(TypeId::of::<T>(), Box::new(value));
            });
        }
    }

    /// 取全局值；未设置返回 `None`。
    pub(crate) fn get(&self) -> Option<T> {
        #[cfg(not(test))]
        {
            self.production.get().cloned()
        }
        #[cfg(test)]
        {
            TEST_SLOTS.with(|slots| {
                slots
                    .borrow()
                    .get(&TypeId::of::<T>())
                    .and_then(|value| value.downcast_ref::<T>())
                    .cloned()
            })
        }
    }

    /// 清空当前线程的全局值（测试期专用：生产期的单例进程内只写一次）。
    #[cfg(test)]
    pub(crate) fn clear(&self) {
        TEST_SLOTS.with(|slots| {
            slots.borrow_mut().remove(&TypeId::of::<T>());
        });
    }
}

/// RAII guard：drop 时清空当前线程的全局值。
///
/// 测试辅助：设置当前线程的全局实例后返回 guard，drop 时自动复原——
/// 测试之间不会互相污染。
#[cfg(test)]
pub(crate) struct TestGuard<T: Clone + 'static> {
    global: &'static Global<T>,
}

#[cfg(test)]
impl<T: Clone + 'static> TestGuard<T> {
    /// 绑定到某个全局槽：drop 时清空该槽的当前线程值。
    pub(crate) fn new(global: &'static Global<T>) -> Self {
        Self { global }
    }
}

#[cfg(test)]
impl<T: Clone + 'static> Drop for TestGuard<T> {
    fn drop(&mut self) {
        self.global.clear();
    }
}
