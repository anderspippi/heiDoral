use parking_lot::RwLock;
use std::collections::HashMap;
use std::convert::TryInto;
use std::ops::Index;
use std::sync::Arc;

use dora_parser::ast;
use dora_parser::interner::Name;
use dora_parser::lexer::position::Position;

use crate::language::sem_analysis::{
    namespace_path, FctDefinitionId, NamespaceDefinitionId, TypeParam, TypeParamDefinition,
    TypeParamId,
};
use crate::language::ty::{SourceType, SourceTypeArray};
use crate::utils::Id;
use crate::vm::{ClassInstanceId, FileId, VM};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TraitDefinitionId(u32);

impl TraitDefinitionId {
    pub fn to_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u32> for TraitDefinitionId {
    fn from(data: u32) -> TraitDefinitionId {
        TraitDefinitionId(data)
    }
}

impl Id for TraitDefinition {
    type IdType = TraitDefinitionId;

    fn id_to_usize(id: TraitDefinitionId) -> usize {
        id.0 as usize
    }

    fn usize_to_id(value: usize) -> TraitDefinitionId {
        TraitDefinitionId(value.try_into().unwrap())
    }

    fn store_id(value: &mut TraitDefinition, id: TraitDefinitionId) {
        value.id = Some(id);
    }
}

#[derive(Debug)]
pub struct TraitDefinition {
    pub id: Option<TraitDefinitionId>,
    pub file_id: FileId,
    pub namespace_id: NamespaceDefinitionId,
    pub is_pub: bool,
    pub ast: Arc<ast::Trait>,
    pub pos: Position,
    pub name: Name,
    pub type_params: Vec<TypeParam>,
    pub type_params2: TypeParamDefinition,
    pub methods: Vec<FctDefinitionId>,
    pub instance_names: HashMap<Name, FctDefinitionId>,
    pub static_names: HashMap<Name, FctDefinitionId>,
    pub vtables: RwLock<HashMap<SourceTypeArray, ClassInstanceId>>,
}

impl TraitDefinition {
    pub fn id(&self) -> TraitDefinitionId {
        self.id.expect("id missing")
    }

    pub fn name(&self, vm: &VM) -> String {
        namespace_path(vm, self.namespace_id, self.name)
    }

    pub fn name_with_params(&self, vm: &VM, type_list: &SourceTypeArray) -> String {
        let name = namespace_path(vm, self.namespace_id, self.name);

        if type_list.len() > 0 {
            let type_list = type_list
                .iter()
                .map(|p| p.name(vm))
                .collect::<Vec<_>>()
                .join(", ");

            format!("{}[{}]", name, type_list)
        } else {
            name.to_string()
        }
    }

    pub fn find_method(&self, vm: &VM, name: Name, is_static: bool) -> Option<FctDefinitionId> {
        for &method in &self.methods {
            let method = vm.fcts.idx(method);
            let method = method.read();

            if method.name == name && method.is_static == is_static {
                return Some(method.id);
            }
        }

        None
    }

    pub fn find_method_with_replace(
        &self,
        vm: &VM,
        is_static: bool,
        name: Name,
        replace: Option<SourceType>,
        args: &[SourceType],
    ) -> Option<FctDefinitionId> {
        for &method in &self.methods {
            let method = vm.fcts.idx(method);
            let method = method.read();

            if method.name == name
                && method.is_static == is_static
                && params_match(replace.clone(), method.params_without_self(), args)
            {
                return Some(method.id);
            }
        }

        None
    }

    pub fn type_param(&self, id: TypeParamId) -> &TypeParam {
        &self.type_params[id.to_usize()]
    }
}

struct TraitType {
    trait_id: TraitDefinitionId,
    type_params: SourceTypeArray,
}

fn params_match(
    replace: Option<SourceType>,
    trait_args: &[SourceType],
    args: &[SourceType],
) -> bool {
    if trait_args.len() != args.len() {
        return false;
    }

    for (ind, ty) in trait_args.iter().enumerate() {
        let ty = ty.clone();
        let other = args[ind].clone();

        let found = if ty.is_self() {
            replace.is_none() || replace.clone().unwrap() == other
        } else {
            ty == other
        };

        if !found {
            return false;
        }
    }

    true
}

impl Index<TraitDefinitionId> for Vec<RwLock<TraitDefinition>> {
    type Output = RwLock<TraitDefinition>;

    fn index(&self, index: TraitDefinitionId) -> &RwLock<TraitDefinition> {
        &self[index.0 as usize]
    }
}
