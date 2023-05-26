//! Many simulations require entities to be positioned in relative, rather than absolute, terms. For
//! example, a magazine might be parented to a gun, which might be parented to the character holding
//! it, which might be parented to a boat they're standing in. Expressing relative positions
//! directly in a component makes computing correct absolute transforms easy and fast.

use hecs::*;

/// Component of entities that are positioned relative to a parent entity
struct Parent {
    /// Parent entity
    entity: Entity,
    /// Converts child-relative coordinates to parent-relative coordinates
    from_child: Transform,
}

fn main() {
    let mut world = World::new();

    // Spawn entities with no parent
    let root = world.spawn((Transform(3, 4),));
    let _other_root = world.spawn((Transform(1, 2),));

    // Spawn some child entities, including dummy transform components that will later be
    // overwritten with derived absolute transforms
    let child = world.spawn((
        Parent {
            entity: root,
            from_child: Transform(1, 1),
        },
        Transform::default(),
    ));
    let _other_child = world.spawn((
        Parent {
            entity: root,
            from_child: Transform(0, 0),
        },
        Transform::default(),
    ));
    let grandchild = world.spawn((
        Parent {
            entity: child,
            from_child: Transform(-1, 0),
        },
        Transform::default(),
    ));

    evaluate_relative_transforms(&mut world);

    // Child entities' transforms are derived recursively from their relationship to their parent
    assert_eq!(*world.get::<&Transform>(child).unwrap(), Transform(4, 5));
    assert_eq!(
        *world.get::<&Transform>(grandchild).unwrap(),
        Transform(3, 5)
    );

    // Moving a parent and re-evaluating moves its children
    *world.get::<&mut Transform>(root).unwrap() = Transform(2, 2);
    evaluate_relative_transforms(&mut world);
    assert_eq!(*world.get::<&Transform>(child).unwrap(), Transform(3, 3));
    assert_eq!(
        *world.get::<&Transform>(grandchild).unwrap(),
        Transform(2, 3)
    );
}

/// Update absolute transforms based on relative transforms
fn evaluate_relative_transforms(world: &mut World) {
    // Construct a view for efficient random access into the set of all entities that have
    // parents. Views allow work like dynamic borrow checking or component storage look-up to be
    // done once rather than per-entity as in `World::get`.
    let mut parents = world.query::<&Parent>();
    let parents = parents.view();

    // View of entities that don't have parents, i.e. roots of the transform hierarchy
    let mut roots = world.query::<&Transform>().without::<&Parent>();
    let roots = roots.view();

    // This query can coexist with the `roots` view without illegal aliasing of `Transform`
    // references because the inclusion of `&Parent` in the query, and its exclusion from the view,
    // guarantees that they will never overlap. Similarly, it can coexist with `parents` because
    // that view does not reference `Transform`s at all.
    for (_entity, (parent, absolute)) in world.query::<(&Parent, &mut Transform)>().iter() {
        // Walk the hierarchy from this entity to the root, accumulating the entity's absolute
        // transform. This does a small amount of redundant work for intermediate levels of deeper
        // hierarchies, but unlike a top-down traversal, avoids tracking entity child lists and is
        // cache-friendly.
        let mut relative = parent.from_child;
        let mut ancestor = parent.entity;
        while let Some(next) = parents.get(ancestor) {
            relative = next.from_child * relative;
            ancestor = next.entity;
        }
        // The `while` loop terminates when `ancestor` cannot be found in `parents`, i.e. when it
        // does not have a `Parent` component, and is therefore necessarily a root.
        *absolute = *roots.get(ancestor).unwrap() * relative;
    }
}

/// 2D translation
// In practice this would usually also include rotation, or even be a general homogeneous matrix
#[derive(Debug, Copy, Clone, Default, Eq, PartialEq)]
struct Transform(i32, i32);

impl std::ops::Mul for Transform {
    type Output = Transform;

    fn mul(self, rhs: Self) -> Transform {
        Transform(self.0 + rhs.0, self.1 + rhs.1)
    }
}
