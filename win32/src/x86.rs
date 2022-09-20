use std::collections::HashMap;

use anyhow::bail;
use tsify::Tsify;

#[derive(Tsify)]
pub struct Registers {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,

    pub esp: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,

    pub eip: u32,

    pub cs: u16,
    pub ds: u16,
    pub es: u16,
    pub fs: u16,
    pub gs: u16,
    pub ss: u16,
}
impl Registers {
    fn new() -> Self {
        Registers {
            eax: 0,
            ebx: 0,
            ecx: 0,
            edx: 0,
            esp: 0,
            ebp: 0,
            esi: 0,
            edi: 0,
            eip: 0,
            cs: 0,
            ds: 0,
            es: 0,
            fs: 0,
            gs: 0,
            ss: 0,
        }
    }

    fn get(&self, name: iced_x86::Register) -> u32 {
        match name {
            iced_x86::Register::None => 0,
            iced_x86::Register::EAX => self.eax,
            iced_x86::Register::EBX => self.ebx,
            iced_x86::Register::ECX => self.ecx,
            iced_x86::Register::EDX => self.edx,
            iced_x86::Register::ESP => self.esp,
            iced_x86::Register::EBP => self.ebp,
            iced_x86::Register::ESI => self.esi,
            iced_x86::Register::EDI => self.edi,
            /*            iced_x86::Register::CS => self.cs,
            iced_x86::Register::DS => self.ds,
            iced_x86::Register::ES => self.es,
            iced_x86::Register::FS => self.fs,
            iced_x86::Register::SS => self.ss,
            iced_x86::Register::GS => self.gs, */
            _ => todo!(),
        }
    }
    fn set(&mut self, name: iced_x86::Register, value: u32) {
        match name {
            iced_x86::Register::EAX => self.eax = value,
            iced_x86::Register::EBX => self.ebx = value,
            iced_x86::Register::ECX => self.ecx = value,
            iced_x86::Register::EDX => self.edx = value,
            iced_x86::Register::ESP => self.esp = value,
            iced_x86::Register::EBP => self.ebp = value,
            iced_x86::Register::ESI => self.esi = value,
            iced_x86::Register::EDI => self.edi = value,
            /*            iced_x86::Register::CS => self.cs,
            iced_x86::Register::DS => self.ds,
            iced_x86::Register::ES => self.es,
            iced_x86::Register::FS => self.fs,
            iced_x86::Register::SS => self.ss,
            iced_x86::Register::GS => self.gs, */
            _ => todo!(),
        }
    }
}

pub struct X86 {
    pub mem: Vec<u8>,
    pub regs: Registers,
    // XXX PE base address, needed for winapi impls; we'll need some win32 system state bit.
    pub base: u32,
    pub imports: HashMap<u32, Option<fn(&mut X86)>>,
}
impl X86 {
    pub fn new() -> Self {
        X86 {
            mem: Vec::new(),
            regs: Registers::new(),
            base: 0,
            imports: HashMap::new(),
        }
    }

    fn write_u32(&mut self, offset: u32, value: u32) {
        let offset = offset as usize;
        self.mem[offset] = (value >> 0) as u8;
        self.mem[offset + 1] = (value >> 8) as u8;
        self.mem[offset + 2] = (value >> 16) as u8;
        self.mem[offset + 3] = (value >> 24) as u8;
    }

    pub fn read_u32(&self, offset: u32) -> u32 {
        let offset = offset as usize;
        ((self.mem[offset] as u32) << 0)
            | ((self.mem[offset + 1] as u32) << 8)
            | ((self.mem[offset + 2] as u32) << 16)
            | ((self.mem[offset + 3] as u32) << 24)
    }

    pub fn push(&mut self, value: u32) {
        self.regs.esp -= 4;
        self.write_u32(self.regs.esp, value);
    }

    pub fn pop(&mut self) -> u32 {
        let value = self.read_u32(self.regs.esp);
        self.regs.esp += 4;
        value
    }

    /// Compute the address found in instructions that reference memory, e.g.
    ///   mov [eax+03h],...
    fn addr(&self, instr: &iced_x86::Instruction) -> u32 {
        assert!(instr.memory_index_scale() == 1);
        self.regs.get(instr.memory_index()) + instr.memory_displacement32()
    }

    fn run(&mut self, instr: &iced_x86::Instruction) -> anyhow::Result<()> {
        self.regs.eip = instr.next_ip() as u32;
        match instr.code() {
            iced_x86::Code::Enterd_imm16_imm8 => {
                self.push(self.regs.ebp);
                self.regs.ebp = self.regs.esp;
                self.regs.esp -= instr.immediate16() as u32;
            }

            iced_x86::Code::Call_rel32_32 => {
                self.push(self.regs.eip);
                self.regs.eip = instr.near_branch32();
            }
            iced_x86::Code::Call_rm32 => {
                // call dword ptr [addr]
                assert!(instr.memory_index() == iced_x86::Register::None);
                let target = self.read_u32(self.addr(instr));
                match self.imports.get(&target) {
                    Some(handler) => match handler {
                        Some(handler) => handler(self),
                        None => log::error!("unimplemented import: {:x}", target),
                    },
                    None => {
                        self.push(self.regs.eip);
                        self.regs.eip = target;
                    }
                };
            }
            iced_x86::Code::Retnd => self.regs.eip = self.pop(),

            iced_x86::Code::Jmp_rel32_32 => {
                self.regs.eip = instr.near_branch32();
            }

            iced_x86::Code::Pushd_imm8 => self.push(instr.immediate8to32() as u32),
            iced_x86::Code::Pushd_imm32 => self.push(instr.immediate32()),
            iced_x86::Code::Push_r32 => self.push(self.regs.get(instr.op0_register())),
            iced_x86::Code::Push_rm32 => {
                // push [eax+10h]
                let value = self
                    .read_u32(self.addr(instr));
                self.push(value);
            }

            iced_x86::Code::Pop_r32 => {
                let value = self.pop();
                self.regs.set(instr.op0_register(), value);
            }

            iced_x86::Code::Mov_rm32_imm32 => {
                // mov dword ptr [x], y
                self.write_u32(self.addr(instr), instr.immediate32());
            }
            iced_x86::Code::Mov_moffs32_EAX => {
                // mov [x],eax
                self.write_u32(self.addr(instr), self.regs.eax);
            }
            iced_x86::Code::Mov_EAX_moffs32 => {
                // mov eax,[x]
                self.regs.eax = self.read_u32(self.addr(instr));
            }
            iced_x86::Code::Mov_rm32_r32 => {
                assert!(instr.op0_kind() == iced_x86::OpKind::Register);
                self.regs
                    .set(instr.op0_register(), self.regs.get(instr.op1_register()));
            }
            iced_x86::Code::Mov_r32_rm32 => {
                assert!(instr.op1_kind() == iced_x86::OpKind::Register);
                self.regs
                    .set(instr.op0_register(), self.regs.get(instr.op1_register()));
            }

            iced_x86::Code::And_rm32_imm8 => {
                assert!(instr.op0_kind() == iced_x86::OpKind::Register);
                let reg = instr.op0_register();
                self.regs
                    .set(reg, self.regs.get(reg) & instr.immediate8() as u32);
            }

            iced_x86::Code::Sub_rm32_imm32 => {
                assert!(instr.op0_kind() == iced_x86::OpKind::Register);
                let reg = instr.op0_register();
                self.regs.set(reg, self.regs.get(reg) - instr.immediate32());
            }

            iced_x86::Code::Lea_r32_m => {
                // lea eax,[esp+10h]
                self.regs.set(instr.op0_register(), self.addr(instr));
            }

            code => {
                self.regs.eip -= instr.len() as u32;
                bail!("unhandled instruction {:?}", code);
            }
        }
        Ok(())
    }

    pub fn step(&mut self) -> anyhow::Result<()> {
        let mut decoder = iced_x86::Decoder::with_ip(
            32,
            &self.mem[self.regs.eip as usize..],
            self.regs.eip as u64,
            iced_x86::DecoderOptions::NONE,
        );
        self.run(&decoder.decode())
    }
}
