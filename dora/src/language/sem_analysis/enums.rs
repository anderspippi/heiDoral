use std::collections::hash_map::HashMap;
use std::convert::TryInto;
use std::ops::Index;
use std::sync::Arc;

use parking_lot::RwLock;

use dora_parser::ast;
use dora_parser::interner::Name;
use dora_parser::lexer::position::Position;

use crate::language::sem_analysis::{
    extension_matches, impl_matches, namespace_path, Candidate, ExtensionDefinitionId,
    ImplDefinitionId, NamespaceId, TypeParam, TypeParamDefinition, TypeParamId,
};
use crate::language::ty::{SourceType, SourceTypeArray};
use crate::utils::Id;
use crate::vm::{EnumInstanceId, FileId, VM};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnumDefinitionId(u32);

impl EnumDefinitionId {
    pub fn to_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<usize> for EnumDefinitionId {
    fn from(data: usize) -> EnumDefinitionId {
        EnumDefinitionId(data.try_into().unwrap())
    }
}

impl Index<EnumDefinitionId> for Vec<RwLock<EnumDefinition>> {
    type Output = RwLock<EnumDefinition>;

    fn index(&self, index: EnumDefinitionId) -> &RwLock<EnumDefinition> {
        &self[index.0 as usize]
    }
}

impl Id for EnumDefinition {
    type IdType = EnumDefinitionId;

    fn id_to_usize(id: EnumDefinitionId) -> usize {
        id.0 as usize
    }

    fn usize_to_id(value: usize) -> EnumDefinitionId {
        EnumDefinitionId(value.try_into().unwrap())
    }

    fn store_id(value: &mut EnumDefinition, id: EnumDefinitionId) {
        value.id = id;
    }
}

#[derive(Debug)]
pub struct EnumDefinition {
    pub id: EnumDefinitionId,
    pub file_id: FileId,
    pub namespace_id: NamespaceId,
    pub ast: Arc<ast::Enum>,
    pub pos: Position,
    pub name: Name,
    pub is_pub: bool,
    pub type_params: Vec<TypeParam>,
    pub type_params2: TypeParamDefinition,
    pub variants: Vec<EnumVariant>,
    pub name_to_value: HashMap<Name, u32>,
    pub impls: Vec<ImplDefinitionId>,
    pub extensions: Vec<ExtensionDefinitionId>,
    pub specializations: RwLock<HashMap<SourceTypeArray, EnumInstanceId>>,
    pub simple_enumeration: bool,
}

impl EnumDefinition {
    pub fn type_param(&self, id: TypeParamId) -> &TypeParam {
        &self.type_params[id.to_usize()]
    }

    pub fn name(&self, vm: &VM) -> String {
        namespace_path(vm, self.namespace_id, self.name)
    }

    pub fn name_with_params(&self, vm: &VM, type_list: &SourceTypeArray) -> String {
        let name = vm.interner.str(self.name);

        if type_list.len() > 0 {
            let type_list = type_list
                .iter()
                .map(|p| p.name_enum(vm, self))
                .collect::<Vec<_>>()
                .join(", ");

            format!("{}[{}]", name, type_list)
        } else {
            name.to_string()
        }
    }
}

#[derive(Debug)]
pub struct EnumVariant {
    pub id: usize,
    pub name: Name,
    pub types: Vec<SourceType>,
}

pub fn find_methods_in_enum(
    vm: &VM,
    object_type: SourceType,
    type_param_defs: &[TypeParam],
    type_param_defs2: Option<&TypeParamDefinition>,
    name: Name,
    is_static: bool,
) -> Vec<Candidate> {
    let enum_id = object_type.enum_id().unwrap();
    let xenum = vm.enums.idx(enum_id);
    let xenum = xenum.read();

    for &extension_id in &xenum.extensions {
        if let Some(bindings) = extension_matches(
            vm,
            object_type.clone(),
            type_param_defs,
            type_param_defs2,
            extension_id,
        ) {
            let extension = vm.extensions[extension_id].read();

            let table = if is_static {
                &extension.static_names
            } else {
                &extension.instance_names
            };

            if let Some(&fct_id) = table.get(&name) {
                return vec![Candidate {
                    object_type: object_type.clone(),
                    container_type_params: bindings,
                    fct_id,
                }];
            }
        }
    }

    let mut candidates = Vec::new();

    for &impl_id in &xenum.impls {
        if let Some(bindings) = impl_matches(
            vm,
            object_type.clone(),
            type_param_defs,
            type_param_defs2,
            impl_id,
        ) {
            let ximpl = vm.impls[impl_id].read();

            let table = if is_static {
                &ximpl.static_names
            } else {
                &ximpl.instance_names
            };

            if let Some(&method_id) = table.get(&name) {
                candidates.push(Candidate {
                    object_type: object_type.clone(),
                    container_type_params: bindings.clone(),
                    fct_id: method_id,
                });
            }
        }
    }

    candidates
}
