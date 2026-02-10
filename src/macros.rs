// src/macros.rs
#[macro_export]
macro_rules! task {
    (|$cx:ident| $body:block) => {
        $crate::FnTask(move |$cx| $body)
    };
}

#[macro_export]
macro_rules! batch_spawn {
    ($scope:expr, $( $name:expr => $task:expr ),+ $(,)?) => {{
        let mut ids = ::std::vec::Vec::new();
        $(
            ids.push($scope.spawn_named($name, $task));
        )+
        ids
    }};
}
