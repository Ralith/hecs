use hecs::Component;

#[derive(Component)]
struct NotSend(std::rc::Rc<()>);

#[derive(Component)]
struct NotStatic<'a>(&'a i32);

fn main() {}
