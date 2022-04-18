use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::language::error::msg::SemError;
use crate::language::sem_analysis::{
    ClassDefinition, ClassDefinitionId, FctDefinitionId, SemAnalysis,
};

pub fn check(sa: &SemAnalysis) {
    let mut abstract_methods: HashMap<ClassDefinitionId, Rc<Vec<FctDefinitionId>>> = HashMap::new();

    for cls in sa.classes.iter() {
        let cls = cls.read();

        // we are only interested in non-abstract classes
        // with abstract super classes

        if cls.is_abstract {
            continue;
        }

        if let Some(parent_class) = cls.parent_class.clone() {
            let super_cls_id = parent_class.cls_id().expect("no class");
            let super_cls = sa.classes.idx(super_cls_id);
            let super_cls = super_cls.read();

            if super_cls.is_abstract {
                check_abstract(sa, &*cls, &*super_cls, &mut abstract_methods);
            }
        }
    }
}

pub fn check_abstract(
    sa: &SemAnalysis,
    cls: &ClassDefinition,
    super_cls: &ClassDefinition,
    abstract_methods: &mut HashMap<ClassDefinitionId, Rc<Vec<FctDefinitionId>>>,
) {
    assert!(!cls.is_abstract);
    assert!(super_cls.is_abstract);

    let mtds = find_abstract_methods(sa, super_cls, abstract_methods);
    let mut overrides = HashSet::new();

    for &mtd in &cls.methods {
        let mtd = sa.fcts.idx(mtd);
        let mtd = mtd.read();

        if let Some(overrides_mtd) = mtd.overrides {
            overrides.insert(overrides_mtd);
        }
    }

    for &mtd in mtds.iter() {
        if !overrides.contains(&mtd) {
            let mtd = sa.fcts.idx(mtd);
            let mtd = mtd.read();

            let mtd_cls = sa.classes.idx(mtd.parent.cls_id());
            let mtd_cls = mtd_cls.read();
            let cls_name = sa.interner.str(mtd_cls.name).to_string();
            let mtd_name = sa.interner.str(mtd.name).to_string();

            sa.diag.lock().report(
                cls.file_id,
                cls.pos,
                SemError::MissingAbstractOverride(cls_name, mtd_name),
            );
        }
    }
}

fn find_abstract_methods(
    sa: &SemAnalysis,
    cls: &ClassDefinition,
    abstract_methods: &mut HashMap<ClassDefinitionId, Rc<Vec<FctDefinitionId>>>,
) -> Rc<Vec<FctDefinitionId>> {
    assert!(cls.is_abstract);

    if let Some(mtds) = abstract_methods.get(&cls.id()) {
        return mtds.clone();
    }

    let mut abstracts: Vec<FctDefinitionId> = Vec::new();
    let mut overrides: HashSet<FctDefinitionId> = HashSet::new();

    for &mtd in &cls.methods {
        let mtd = sa.fcts.idx(mtd);
        let mtd = mtd.read();

        if mtd.is_abstract {
            abstracts.push(mtd.id());
        }

        if let Some(override_mtd) = mtd.overrides {
            overrides.insert(override_mtd);
        }
    }

    if let Some(parent_class) = cls.parent_class.clone() {
        let super_cls_id = parent_class.cls_id().expect("no class");
        let super_cls = sa.classes.idx(super_cls_id);
        let super_cls = super_cls.read();

        if super_cls.is_abstract {
            let super_abstracts = find_abstract_methods(sa, &*super_cls, abstract_methods);

            for &mtd in super_abstracts.iter() {
                if !overrides.contains(&mtd) {
                    abstracts.push(mtd);
                }
            }
        }
    }

    let ret = Rc::new(abstracts);
    abstract_methods.insert(cls.id(), ret.clone());

    ret
}

#[cfg(test)]
mod tests {
    use crate::language::error::msg::SemError;
    use crate::language::tests::{err, ok, pos};

    #[test]
    fn test_abstract_class_without_abstract_methods() {
        ok("@open @abstract class A class B: A");
    }

    #[test]
    fn test_override_abstract_method() {
        ok("@open @abstract class A { @abstract fn foo(); }
            class B: A { @override fn foo() {} }");
    }

    #[test]
    fn test_override_abstract_method_in_super_class() {
        ok("@open @abstract class A { @abstract fn foo(); }
            @open @abstract class B: A { @override fn foo() {} }
            class C: B { }");
    }

    #[test]
    fn test_missing_abstract_override() {
        err(
            "@open @abstract class A { @abstract fn foo(); }
            class B: A { }",
            pos(2, 13),
            SemError::MissingAbstractOverride("A".into(), "foo".into()),
        );
    }

    #[test]
    fn test_missing_abstract_override_indirect() {
        err(
            "@open @abstract class A { @abstract fn foo(); }
            @open @abstract class B: A {}
            class C: B { }",
            pos(3, 13),
            SemError::MissingAbstractOverride("A".into(), "foo".into()),
        );
    }
}
