use std::ptr;

use crate::handle::{handle, Handle};
use crate::language::sem_analysis::FctParent;
use crate::object::{alloc, Array, Int32Array, Ref, Stacktrace, StacktraceElement, Str};
use crate::threads::current_thread;
use crate::vm::{get_vm, CodeId, CodeKind, VM};

pub struct NativeStacktrace {
    elems: Vec<StackElem>,
}

impl NativeStacktrace {
    pub fn new() -> NativeStacktrace {
        NativeStacktrace { elems: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.elems.len()
    }

    pub fn push_entry(&mut self, fct_id: CodeId, lineno: i32) {
        self.elems.push(StackElem { fct_id, lineno });
    }

    pub fn dump(&self, vm: &VM) {
        let frames = self.elems.len();
        for (ind, elem) in self.elems.iter().enumerate() {
            let code = vm.code.idx(elem.fct_id);
            let fct_id = code.fct_id();
            let fct = vm.fcts.idx(fct_id);
            let fct = fct.read();
            let name = fct.name_with_params(vm);
            print!("{}: {}: ", frames - ind, name);

            if elem.lineno == 0 {
                println!("?");
            } else {
                println!("{}", elem.lineno);
            }
        }
    }

    pub fn dump_err(&self, vm: &VM) {
        let frames = self.elems.len();
        for (ind, elem) in self.elems.iter().enumerate() {
            let code = vm.code.idx(elem.fct_id);
            let fct_id = code.fct_id();
            let fct = vm.fcts.idx(fct_id);
            let fct = fct.read();
            let name = fct.name_with_params(vm);
            eprint!("{}: {}: ", frames - ind, name);

            if elem.lineno == 0 {
                eprintln!("?");
            } else {
                eprintln!("{}", elem.lineno);
            }
        }
    }
}

struct StackElem {
    fct_id: CodeId,
    lineno: i32,
}

#[repr(C)]
pub struct DoraToNativeInfo {
    // pointer to previous DTN-Info
    pub last: *const DoraToNativeInfo,

    // frame pointer of native stub
    pub fp: usize,

    // some program counter into native stub
    pub pc: usize,
}

impl DoraToNativeInfo {
    pub fn new() -> DoraToNativeInfo {
        DoraToNativeInfo {
            last: ptr::null(),
            fp: 0,
            pc: 0,
        }
    }

    pub fn last_offset() -> i32 {
        offset_of!(DoraToNativeInfo, last) as i32
    }

    pub fn fp_offset() -> i32 {
        offset_of!(DoraToNativeInfo, fp) as i32
    }

    pub fn pc_offset() -> i32 {
        offset_of!(DoraToNativeInfo, pc) as i32
    }
}

pub fn stacktrace_from_last_dtn(vm: &VM) -> NativeStacktrace {
    let mut stacktrace = NativeStacktrace::new();
    frames_from_dtns(&mut stacktrace, vm);
    return stacktrace;
}

fn frames_from_dtns(stacktrace: &mut NativeStacktrace, vm: &VM) {
    let mut dtn_ptr = current_thread().dtn();

    while !dtn_ptr.is_null() {
        let dtn = unsafe { &*dtn_ptr };

        let pc: usize = dtn.pc;
        let fp: usize = dtn.fp;

        frames_from_pc(stacktrace, vm, pc, fp);

        dtn_ptr = dtn.last
    }
}

fn frames_from_pc(stacktrace: &mut NativeStacktrace, vm: &VM, pc: usize, mut fp: usize) {
    if !determine_stack_entry(stacktrace, vm, pc) {
        return;
    }

    while fp != 0 {
        let ra = unsafe { *((fp + 8) as *const usize) };

        if !determine_stack_entry(stacktrace, vm, ra) {
            return;
        }

        fp = unsafe { *(fp as *const usize) };
    }
}

fn determine_stack_entry(stacktrace: &mut NativeStacktrace, vm: &VM, pc: usize) -> bool {
    let code_id = vm.code_map.get(pc.into());

    if let Some(code_id) = code_id {
        let code = vm.code.idx(code_id);
        match code.descriptor() {
            CodeKind::DoraFct(_) => {
                let offset = pc - code.instruction_start().to_usize();
                let position = code
                    .position_for_offset(offset as u32)
                    .expect("position not found for program point");

                stacktrace.push_entry(code_id, position.line as i32);

                true
            }

            CodeKind::NativeStub(fct_id) => {
                let fct = vm.fcts.idx(fct_id);
                let fct = fct.read();

                stacktrace.push_entry(code_id, fct.ast.pos.line as i32);

                true
            }

            CodeKind::TrapStub => true,
            CodeKind::GuardCheckStub => true,
            CodeKind::CompileStub => true,
            CodeKind::AllocStub => true,
            CodeKind::DoraStub => false,

            CodeKind::VerifyStub | CodeKind::SafepointStub => unreachable!(),
        }
    } else {
        println!("no code found at pc = {:x}", pc);
        vm.code_map.dump(vm);
        panic!("invalid stack frame");
    }
}

pub struct ThrowResume {
    pc: usize,
    sp: usize,
    fp: usize,
}

pub extern "C" fn retrieve_stack_trace(obj: Handle<Stacktrace>) {
    let vm = get_vm();
    set_backtrace(vm, obj, true);
}

pub extern "C" fn stack_element(obj: Handle<Stacktrace>, ind: i32) -> Ref<StacktraceElement> {
    let vm = get_vm();
    let array = obj.backtrace;

    let ind = ind as usize * 2;

    let lineno = array.get_at(ind);
    let fct_id = array.get_at(ind + 1);
    let cls_def_id = vm.known.stack_trace_element(vm);

    let ste: Ref<StacktraceElement> = alloc(vm, cls_def_id).cast();
    let mut ste = handle(ste);
    ste.line = lineno;

    let code_id: CodeId = (fct_id as usize).into();
    let code = vm.code.idx(code_id);
    let fct = vm.fcts.idx(code.fct_id());
    let fct = fct.read();
    let name = fct.name_with_params(vm);
    ste.name = Str::from_buffer(vm, name.as_bytes());

    ste.direct()
}

fn set_backtrace(vm: &VM, mut obj: Handle<Stacktrace>, via_retrieve: bool) {
    let stacktrace = stacktrace_from_last_dtn(vm);
    let mut skip = 0;

    let mut skip_retrieve_stack = false;
    let mut skip_constructor = false;

    // ignore every element until first not inside susubclass of Stacktrace (ctor of Exception)
    if via_retrieve {
        for elem in stacktrace.elems.iter() {
            let code_id = elem.fct_id.idx().into();
            let code = vm.code.idx(code_id);
            let fct_id = code.fct_id();
            let fct = vm.fcts.idx(fct_id);
            let fct = fct.read();

            if !skip_retrieve_stack {
                let stacktrace_cls = vm.classes.idx(vm.known.classes.stacktrace());
                let stacktrace_cls = stacktrace_cls.read();
                let retrieve_stacktrace_fct_id = stacktrace_cls
                    .find_method(vm, vm.interner.intern("retrieveStacktrace"), false)
                    .expect("retrieveStacktrace not found in Stacktrace");

                if retrieve_stacktrace_fct_id == fct_id {
                    skip += 1;
                    continue;
                } else {
                    skip_retrieve_stack = true;
                }
            }

            if !skip_constructor {
                assert!(skip_retrieve_stack);
                if let FctParent::Class(owner_class) = fct.parent {
                    if fct.is_constructor {
                        let throw_object_cls_id = (&obj.header)
                            .vtbl()
                            .class_def()
                            .cls_id
                            .expect("no corresponding class");

                        if throw_object_cls_id == owner_class {
                            skip += 1;
                            skip_constructor = true;
                            break;
                        }

                        let throw_object_cls = vm.classes.idx(throw_object_cls_id);
                        let throw_object_cls = throw_object_cls.read();

                        if throw_object_cls.subclass_from(vm, owner_class) {
                            skip += 1;
                            continue;
                        }
                    } else {
                        skip_constructor = true;
                        break;
                    }
                } else {
                    skip_constructor = true;
                    break;
                }
            }
        }
        assert!(skip_constructor);
    }

    let len = stacktrace.len() - skip;

    let cls_id = vm.known.int_array(vm);
    let array: Ref<Int32Array> = Array::alloc(vm, len * 2, 0, cls_id);
    let mut array = handle(array);
    let mut i = 0;

    for elem in stacktrace.elems.iter().skip(skip) {
        array.set_at(i, elem.lineno);
        array.set_at(i + 1, elem.fct_id.idx() as i32);
        i += 2;
    }
    obj.backtrace = array.direct();
}
