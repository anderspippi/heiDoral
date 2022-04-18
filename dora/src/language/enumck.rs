use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::RwLock;

use dora_parser::ast;
use dora_parser::ast::TypeParam;

use crate::language::error::msg::SemError;
use crate::language::sem_analysis::{EnumDefinition, EnumVariant, SourceFileId, TypeParamId};
use crate::language::sym::{NestedSymTable, Sym};
use crate::language::ty::SourceType;
use crate::language::{read_type, AllowSelf, TypeParamContext};
use crate::vm::SemAnalysis;

pub fn check(sa: &SemAnalysis) {
    for enum_ in sa.enums.iter() {
        let ast = enum_.read().ast.clone();

        let mut enumck = EnumCheck {
            sa,
            file_id: enum_.read().file_id,
            ast: &ast,
            enum_: &enum_,
        };

        enumck.check();
    }
}

struct EnumCheck<'x> {
    sa: &'x SemAnalysis,
    file_id: SourceFileId,
    ast: &'x Arc<ast::Enum>,
    enum_: &'x RwLock<EnumDefinition>,
}

impl<'x> EnumCheck<'x> {
    fn check(&mut self) {
        let mut symtable = NestedSymTable::new(self.sa, self.enum_.read().module_id);

        symtable.push_level();

        if let Some(ref type_params) = self.ast.type_params {
            self.check_type_params(type_params, &mut symtable);
        }

        let mut variant_idx: usize = 0;
        let mut simple_enumeration = true;

        for value in &self.ast.variants {
            let mut types: Vec<SourceType> = Vec::new();

            if let Some(ref variant_types) = value.types {
                for ty in variant_types {
                    let variant_ty = read_type(
                        self.sa,
                        &symtable,
                        self.file_id.into(),
                        ty,
                        TypeParamContext::Enum(self.enum_.read().id()),
                        AllowSelf::No,
                    )
                    .unwrap_or(SourceType::Error);
                    types.push(variant_ty);
                }
            }

            if types.len() > 0 {
                simple_enumeration = false;
            }

            self.enum_.write().variants[variant_idx].types = types;
            variant_idx += 1;
        }

        self.enum_.write().simple_enumeration = simple_enumeration;

        symtable.pop_level();
    }

    fn check_type_params(&mut self, type_params: &[TypeParam], symtable: &mut NestedSymTable) {
        if type_params.len() > 0 {
            let mut names = HashSet::new();
            let mut type_param_id = 0;
            let mut params = Vec::new();

            for type_param in type_params {
                if !names.insert(type_param.name) {
                    let name = self.sa.interner.str(type_param.name).to_string();
                    let msg = SemError::TypeParamNameNotUnique(name);
                    self.sa
                        .diag
                        .lock()
                        .report(self.file_id, type_param.pos, msg);
                }

                params.push(SourceType::TypeParam(TypeParamId(type_param_id)));

                for bound in &type_param.bounds {
                    let ty = read_type(
                        self.sa,
                        symtable,
                        self.file_id,
                        bound,
                        TypeParamContext::Enum(self.enum_.read().id()),
                        AllowSelf::No,
                    );

                    match ty {
                        Some(SourceType::Trait(trait_id, _)) => {
                            if !self.enum_.write().type_params[type_param_id]
                                .trait_bounds
                                .insert(trait_id)
                            {
                                let msg = SemError::DuplicateTraitBound;
                                self.sa
                                    .diag
                                    .lock()
                                    .report(self.file_id, type_param.pos, msg);
                            }
                        }

                        None => {
                            // unknown type, error is already thrown
                        }

                        _ => {
                            let msg = SemError::BoundExpected;
                            self.sa.diag.lock().report(self.file_id, bound.pos(), msg);
                        }
                    }
                }

                let sym = Sym::TypeParam(TypeParamId(type_param_id));
                symtable.insert(type_param.name, sym);
                type_param_id += 1;
            }
        } else {
            let msg = SemError::TypeParamsExpected;
            self.sa.diag.lock().report(self.file_id, self.ast.pos, msg);
        }
    }
}

pub fn check_variants(sa: &SemAnalysis) {
    for enum_ in sa.enums.iter() {
        let mut enum_ = enum_.write();
        let ast = enum_.ast.clone();

        let mut enumck = EnumCheckVariants {
            sa,
            file_id: enum_.file_id,
            ast: &ast,
            enum_: &mut *enum_,
        };

        enumck.check();
    }
}

struct EnumCheckVariants<'x> {
    sa: &'x SemAnalysis,
    file_id: SourceFileId,
    ast: &'x Arc<ast::Enum>,
    enum_: &'x mut EnumDefinition,
}

impl<'x> EnumCheckVariants<'x> {
    fn check(&mut self) {
        let mut next_variant_id: u32 = 0;

        for value in &self.ast.variants {
            let variant = EnumVariant {
                id: next_variant_id as usize,
                name: value.name,
                types: Vec::new(),
            };

            self.enum_.variants.push(variant);
            let result = self.enum_.name_to_value.insert(value.name, next_variant_id);

            if result.is_some() {
                let name = self.sa.interner.str(value.name).to_string();
                self.sa.diag.lock().report(
                    self.enum_.file_id,
                    value.pos,
                    SemError::ShadowEnumValue(name),
                );
            }

            next_variant_id += 1;
        }

        if self.ast.variants.is_empty() {
            self.sa
                .diag
                .lock()
                .report(self.enum_.file_id, self.ast.pos, SemError::NoEnumValue);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::language::error::msg::SemError;
    use crate::language::tests::*;

    #[test]
    fn enum_definitions() {
        err("enum Foo {}", pos(1, 1), SemError::NoEnumValue);
        ok("enum Foo { A, B, C }");
        err(
            "enum Foo { A, A }",
            pos(1, 15),
            SemError::ShadowEnumValue("A".into()),
        );
    }

    #[test]
    fn enum_with_argument() {
        ok("
            enum Foo { A(Int32), B(Float32), C}
            fn give_me_a(): Foo { Foo::A(1I) }
            fn give_me_b(): Foo { Foo::B(2.0F) }
            fn give_me_c(): Foo { Foo::C }

        ");
    }

    #[test]
    fn enum_wrong_type() {
        err(
            "
            enum Foo { A(Int32), B(Float32), C}
            fn give_me_a(): Foo { Foo::A(2.0F) }

        ",
            pos(3, 41),
            SemError::EnumArgsIncompatible(
                "Foo".into(),
                "A".into(),
                vec!["Int32".into()],
                vec!["Float32".into()],
            ),
        );
    }

    #[test]
    fn enum_missing_args() {
        err(
            "
            enum Foo { A(Int32), B(Float32), C}
            fn give_me_a(): Foo { Foo::A }

        ",
            pos(3, 38),
            SemError::EnumArgsIncompatible(
                "Foo".into(),
                "A".into(),
                vec!["Int32".into()],
                Vec::new(),
            ),
        );
    }

    #[test]
    fn enum_unexpected_args() {
        err(
            "
            enum Foo { A(Int32), B(Float32), C}
            fn give_me_c(): Foo { Foo::C(12.0F) }

        ",
            pos(3, 41),
            SemError::EnumArgsIncompatible(
                "Foo".into(),
                "C".into(),
                Vec::new(),
                vec!["Float32".into()],
            ),
        );
    }

    #[test]
    fn enum_parens_but_no_args() {
        err(
            "
            enum Foo { A(Int32), B(Float32), C}
            fn give_me_c(): Foo { Foo::C() }
        ",
            pos(3, 41),
            SemError::EnumArgsNoParens("Foo".into(), "C".into()),
        );
    }

    #[test]
    fn enum_copy() {
        ok("
            enum Foo { A(Int32), B(Float32), C}
            fn foo_test(y: Foo): Foo { let x: Foo = y; x }
        ");
    }

    #[test]
    fn enum_generic() {
        ok("
            enum Foo[T] { One(T), Two }
        ");
    }

    #[test]
    fn enum_with_type_param() {
        ok("trait SomeTrait {} enum MyOption[T: SomeTrait] { None, Some(T) }");
    }

    #[test]
    fn enum_generic_with_failures() {
        err(
            "enum MyOption[] { A, B }",
            pos(1, 1),
            SemError::TypeParamsExpected,
        );

        err(
            "enum MyOption[X, X] { A, B }",
            pos(1, 18),
            SemError::TypeParamNameNotUnique("X".into()),
        );

        err(
            "enum MyOption[X: NonExistingTrait] { A, B }",
            pos(1, 18),
            SemError::UnknownIdentifier("NonExistingTrait".into()),
        );
    }

    #[test]
    fn check_enum_type() {
        err(
            "
                enum MyOption[X] { A, B }
                fn foo(v: MyOption) {}
            ",
            pos(3, 27),
            SemError::WrongNumberTypeParams(1, 0),
        );
    }

    #[test]
    fn check_enum_value() {
        ok("
            enum Foo { A(Int32), B }
            fn foo(): Foo { Foo::A(1I) }
            fn bar(): Foo { Foo::B }
        ");

        err(
            "
            enum Foo { A(Int32), B }
            fn foo(): Foo { Foo::A(true) }
        ",
            pos(3, 35),
            SemError::EnumArgsIncompatible(
                "Foo".into(),
                "A".into(),
                vec!["Int32".into()],
                vec!["Bool".into()],
            ),
        );
    }

    #[test]
    fn check_enum_value_generic() {
        ok("
            enum Foo[T] { A, B }
            fn foo() { let tmp = Foo[String]::B; }
        ");

        err(
            "
            trait SomeTrait {}
            enum Foo[T: SomeTrait] { A, B }
            fn foo() { let tmp = Foo[String]::B; }
        ",
            pos(4, 45),
            SemError::TypeNotImplementingTrait("String".into(), "SomeTrait".into()),
        );
    }

    #[test]
    fn enum_with_generic_argument() {
        ok("
            enum Foo[T] { A(T), B }
            fn foo() { let tmp = Foo[Int32]::A(0I); }
        ");

        err(
            "
            enum Foo[T] { A(T), B }
            fn foo() { let tmp = Foo[Int32]::A(true); }
        ",
            pos(3, 47),
            SemError::EnumArgsIncompatible(
                "Foo".into(),
                "A".into(),
                vec!["T".into()],
                vec!["Bool".into()],
            ),
        );
    }

    #[test]
    fn enum_move_generic() {
        ok("
            enum Foo[T] { A(T), B }
            fn foo(x: Foo[Int32]): Foo[Int32] { x }
        ");

        err(
            "
            enum Foo[T] { A(T), B }
            fn foo(x: Foo[Int32]): Foo[Float32] { x }
        ",
            pos(3, 49),
            SemError::ReturnType("Foo[Float32]".into(), "Foo[Int32]".into()),
        );
    }

    #[test]
    fn enum_nested() {
        ok("
            enum Foo { A(Foo), B }
        ");
    }
}
