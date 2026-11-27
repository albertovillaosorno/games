// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
//
// Boundary-Contract:
// - Owns:
//   - Konami 053248 instruction constructors and analysis metadata.
// - Must-Not:
//   - Decode unsupported instructions as another processor.
// - Allows:
//   - Inputs: Authored instruction descriptors.
//   - Outputs: Portable SLEIGH and XML component text.
//   - Side effects: None beyond allocation of generated text.
// - Split-When:
//   - Instruction families exceed a coherent processor module.
// - Merge-When:
//   - A shared processor generator owns equivalent logic.
// - Summary:
//   - Generate a partial, fail-closed Konami processor language.
// - Description:
//   - Board bank selection is recorded, not automatically mapped.
// - Usage:
//   - Emit with games-build ghidra konami slaspec.
// - Defaults:
//   - No catch-all instruction constructors.

//! Konami 053248 processor description.

use std::fmt::Write as _;

pub(super) const COMPILER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<compiler_spec>
  <global><range space="RAM"/></global>
  <stackpointer register="S" space="RAM" growth="negative"/>
  <default_proto>
    <prototype name="__unknown" extrapop="2" stackshift="2">
      <input>
        <pentry minsize="1" maxsize="1"><register name="A"/></pentry>
        <pentry minsize="1" maxsize="1"><register name="B"/></pentry>
        <pentry minsize="2" maxsize="2"><register name="X"/></pentry>
        <pentry minsize="2" maxsize="2"><register name="Y"/></pentry>
        <pentry minsize="2" maxsize="2"><register name="U"/></pentry>
      </input>
      <output>
        <pentry minsize="1" maxsize="1"><register name="A"/></pentry>
      </output>
      <unaffected><register name="S"/></unaffected>
    </prototype>
  </default_proto>
</compiler_spec>"#;

pub(super) const LANGUAGE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<language_definitions>
  <language processor="Konami053248" endian="big" size="16"
            variant="partial" version="1.0" slafile="konami.sla"
            processorspec="konami.pspec"
            id="Konami053248:BE:16:partial">
    <description>Konami 053248 (partial analysis language)</description>
    <compiler name="conservative" spec="konami.cspec" id="default"/>
  </language>
</language_definitions>"#;

pub(super) const PROCESSOR: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<processor_spec>
  <programcounter register="PC"/>
</processor_spec>"#;

const BASE: &str = r#"# Authored Konami 053248 subset; not Motorola 6809.
define endian=big;
define alignment=1;
define space RAM type=ram_space size=2 default;
define space register type=register_space size=2;
define register offset=0 size=1 [ A B ];
define register offset=0 size=2 [ D ];
define register offset=8 size=1 [ CC DP BANK ];
define register offset=16 size=2 [ PC X Y U S ];

@define C "CC[0,1]"
@define V "CC[1,1]"
@define Z "CC[2,1]"
@define N "CC[3,1]"
@define H "CC[5,1]"

define token opcode(8) op=(0,7);
define token byte(8) imm8=(0,7) simm8=(0,7) signed;
define token word(16) imm16=(0,15) simm16=(0,15) signed;
define token post(8) postbyte=(0,7);

I8: imm8 is imm8 { export *[const]:1 imm8; }
I16: imm16 is imm16 { export *[const]:2 imm16; }
R8: target is simm8 [ target=inst_next+simm8; ]
{ export *:2 target; }
R16: target is simm16 [ target=inst_next+simm16; ]
{ export *:2 target; }

EA: imm16 is postbyte=0x07; imm16
{ export *[const]:2 imm16; }
EA: "["imm16"]" is postbyte=0x0f; imm16
{ local ptr:2 = imm16; export *:2 ptr; }
EA: imm8 is postbyte=0xc4; imm8
{ local addr:2=(zext(DP)<<8)|zext(imm8:1); export addr; }
EA: "["imm8"]" is postbyte=0xcc; imm8
{ local addr:2=(zext(DP)<<8)|zext(imm8:1); export *:2 addr; }
M8: EA is EA { export *:1 EA; }
M16: EA is EA { export *:2 EA; }
ABS: imm16 is postbyte=0x07; imm16
{ export *:2 imm16; }

macro nz(result) {
    $(Z) = result == 0;
    $(N) = result s< 0;
}
macro load(reg,val) {
    reg=val; nz(reg); $(V)=0;
}
macro add(reg,val) {
    local rhs=val; $(C)=carry(reg,rhs); $(V)=scarry(reg,rhs);
    reg=reg+rhs; nz(reg);
}
macro add8(reg,val) {
    local rhs:1=val;
    $(H)=((reg & 0xf)+(rhs & 0xf))>0xf;
    add(reg,rhs);
}
macro sub(reg,val) {
    local rhs=val; $(C)=reg<rhs; $(V)=sborrow(reg,rhs);
    reg=reg-rhs; nz(reg);
}
macro cmp(reg,val) {
    local rhs=val; local result=reg-rhs;
    $(C)=reg<rhs; $(V)=sborrow(reg,rhs); nz(result);
}
macro bit(reg,val) {
    local result=reg&val; nz(result); $(V)=0;
}
macro clear_value(dst) {
    dst=0; $(N)=0; $(Z)=1; $(V)=0; $(C)=0;
}
macro complement(dst) {
    dst=~dst; nz(dst); $(V)=0; $(C)=1;
}
macro negate8(dst) {
    local value:1=dst;
    dst=0-value; nz(dst); $(C)=value!=0; $(V)=value==0x80;
}
macro negate16(dst) {
    local value:2=dst;
    dst=0-value; nz(dst); $(C)=value!=0; $(V)=value==0x8000;
}
macro absolute8(dst) {
    local old:1=dst; local sign:1=old s>>7;
    dst=(old^sign)-sign;
    $(C)=old s<0; $(V)=old==0x80; nz(dst);
}
macro absolute16(dst) {
    local old:2=dst; local sign:2=old s>>15;
    dst=(old^sign)-sign;
    $(C)=old s<0; $(V)=old==0x8000; nz(dst);
}
macro inc8(dst) { $(V)=dst==0x7f; dst=dst+1; nz(dst); }
macro dec8(dst) { $(V)=dst==0x80; dst=dst-1; nz(dst); }
macro inc16(dst) { add(dst,1:2); }
macro dec16(dst) { sub(dst,1:2); }
macro test_value(dst) { nz(dst); $(V)=0; }
macro lsr(dst) { $(C)=(dst&1)!=0; dst=dst>>1; nz(dst); }
macro asr(dst) { $(C)=(dst&1)!=0; dst=dst s>>1; nz(dst); }
macro asl(dst) { add(dst,dst); }
macro ror8(dst) {
    local carryin:1=$(C); $(C)=(dst&1)!=0;
    dst=(dst>>1)|(carryin<<7); nz(dst);
}
macro ror16(dst) {
    local carryin:2=zext($(C)); $(C)=(dst&1)!=0;
    dst=(dst>>1)|(carryin<<15); nz(dst);
}
macro rol8(dst) {
    local old:1=dst; local carryin:1=$(C);
    $(C)=(old&0x80)!=0; dst=(old<<1)|carryin;
    $(V)=((old^dst)&0x80)!=0; nz(dst);
}
macro rol16(dst) {
    local old:2=dst; local carryin:2=zext($(C));
    $(C)=(old&0x8000)!=0; dst=(old<<1)|carryin;
    $(V)=0; nz(dst);
}
macro adc8(dst,val) {
    local left:1=dst; local right:1=val;
    local ci:1=$(C);
    local result:2=zext(left)+zext(right)+zext(ci);
    dst=result:1;
    $(C)=result>0xff;
    $(H)=((left&0xf)+(right&0xf)+ci)>0xf;
    $(V)=((~(left^right)&(left^dst))&0x80)!=0;
    nz(dst);
}
macro sbc8(dst,val) {
    local left:1=dst; local right:1=val;
    local ci:1=$(C);
    local rhs:2=zext(right)+zext(ci);
    dst=left-rhs:1;
    $(C)=zext(left)<rhs;
    $(V)=(((left^right)&(left^dst))&0x80)!=0;
    nz(dst);
}
macro pushret() {
    S=S-2; *:2 S=inst_next;
}

:SETLN I8 is op=0x38; I8 { BANK=I8; }
:SETLN M8 is op=0x39; M8 { BANK=M8; }
:ANDCC I8 is op=0x3c; I8 { CC=CC&I8; }
:ORCC I8 is op=0x3d; I8 { CC=CC|I8; }
:LEAX EA is op=0x08; EA { X=EA; $(Z)=X==0; }
:LEAY EA is op=0x09; EA { Y=EA; $(Z)=Y==0; }
:LEAU EA is op=0x0a; EA { U=EA; }
:LEAS EA is op=0x0b; EA { S=EA; }
:JMP ABS is op=0xa8; ABS { goto ABS; }
:JSR ABS is op=0xa9; ABS { pushret(); call ABS; }
:JMP EA is op=0xa8; EA { goto [EA]; }
:JSR EA is op=0xa9; EA { pushret(); call [EA]; }
:DBJNZ R8 is op=0xac; R8 {
    $(V)=B==0x80; B=B-1; nz(B); if ($(Z)==0) goto R8;
}
:DXJNZ R8 is op=0xad; R8 {
    $(V)=X==0x8000; X=X-1; nz(X); if ($(Z)==0) goto R8;
}
:RTS is op=0x8f { local dest:2=*:2 S; S=S+2; return [dest]; }
:RTI is op=0x9f {
    CC=*:1 S; S=S+1;
    if ((CC&0x80)==0) goto <pc>;
    A=*:1 S; S=S+1;
    B=*:1 S; S=S+1;
    DP=*:1 S; S=S+1;
    X=*:2 S; S=S+2;
    Y=*:2 S; S=S+2;
    U=*:2 S; S=S+2;
<pc>
    local dest:2=*:2 S; S=S+2; return [dest];
}
:BSR R8 is op=0xaa; R8 { pushret(); call R8; }
:LBSR R16 is op=0xab; R16 { pushret(); call R16; }
:NOP is op=0xae { }
:ABX is op=0xb0 { X=X+zext(B); }
:SEX is op=0xb2 { D=sext(B); nz(D); }
:DAA is op=0xb1 {
    local low:1=A&0x0f; local high:1=A&0xf0;
    local adjustment:2=0;
    if ((low<=9) && ($(H)==0)) goto <high_check>;
    adjustment=6;
<high_check>
    if ((high>0x80 && low>9) || high>0x90 || $(C)!=0) goto <adjust_high>;
    goto <adjust>;
<adjust_high>
    adjustment=adjustment|0x60;
<adjust>
    local result:2=zext(A)+adjustment;
    A=result:1; $(V)=0; $(C)=$(C)||(result>0xff); nz(A);
}
:MUL is op=0xb3 {
    local result:2=zext(A)*zext(B);
    D=result; $(Z)=result==0; $(C)=(result&0x80)!=0;
}
:LMUL is op=0xb4 {
    local result:4=zext(X)*zext(Y);
    local upper:4=result>>16;
    X=upper:2; Y=result:2; $(Z)=result==0; $(C)=(result&0x8000)!=0;
}
:DIVX is op=0xb5 {
    local quotient:2=0; local remainder:2=0;
    if (B==0) goto <done>;
    local divisor:2=zext(B);
    quotient=X/divisor; remainder=X%divisor;
<done>
    X=quotient; B=remainder:1;
    $(Z)=quotient==0; $(C)=(quotient&0x80)!=0;
}
:BMOVE is op=0xb6 {
<loop>
    if (U==0) goto <done>;
    *:1 X=*:1 Y;
    X=X+1; Y=Y+1; U=U-1;
    goto <loop>;
<done>
}
:MOVE is op=0xb7 { *:1 X=*:1 Y; X=X+1; Y=Y+1; U=U-1; }
:ABSA is op=0xcc { absolute8(A); }
:ABSB is op=0xcd { absolute8(B); }
:ABSD is op=0xce { absolute16(D); }
:BSET is op=0xcf {
<loop>
    if (U==0) goto <done>;
    *:1 X=A; X=X+1; U=U-1;
    goto <loop>;
<done>
}
:BSETW is op=0xd0 {
<loop>
    if (U==0) goto <done>;
    *:1 X=A; X=X+1;
    *:1 X=B; X=X+1; U=U-1;
    goto <loop>;
<done>
}
"#;

// Opcode assignments and addressing forms checked against the BSD-3-Clause
// MAME Konami processor descriptions, revision
// 89185ed286c7830b4e6daa647d843a929077ecd2. The authored p-code models only the
// explicitly listed instruction subset.

const REGISTERS: [(u8, &str); 4] =
    [(0x20, "X"), (0x30, "Y"), (0x50, "U"), (0x60, "S")];

const BYTE_OPS: [ByteOp; 10] = [
    (0x10, "LD", "load"),
    (0x14, "ADD", "add8"),
    (0x18, "ADC", "adc8"),
    (0x1c, "SUB", "sub"),
    (0x20, "SBC", "sbc8"),
    (0x24, "AND", "and"),
    (0x28, "BIT", "bit"),
    (0x2c, "EOR", "xor"),
    (0x30, "OR", "or"),
    (0x34, "CMP", "cmp"),
];

const WORD_OPS: [WordOp; 12] = [
    (0x40, "LDD", "D", "load"),
    (0x42, "LDX", "X", "load"),
    (0x44, "LDY", "Y", "load"),
    (0x46, "LDU", "U", "load"),
    (0x48, "LDS", "S", "load"),
    (0x4a, "CMPD", "D", "cmp"),
    (0x4c, "CMPX", "X", "cmp"),
    (0x4e, "CMPY", "Y", "cmp"),
    (0x50, "CMPU", "U", "cmp"),
    (0x52, "CMPS", "S", "cmp"),
    (0x54, "ADDD", "D", "add"),
    (0x56, "SUBD", "D", "sub"),
];

const BRANCHES: [(&str, &str); 16] = [
    ("BRA", "1"),
    ("BHI", "!$(C) && !$(Z)"),
    ("BCC", "!$(C)"),
    ("BNE", "!$(Z)"),
    ("BVC", "!$(V)"),
    ("BPL", "!$(N)"),
    ("BGE", "$(N)==$(V)"),
    ("BGT", "!$(Z) && ($(N)==$(V))"),
    ("BRN", "0"),
    ("BLS", "$(C) || $(Z)"),
    ("BCS", "$(C)"),
    ("BEQ", "$(Z)"),
    ("BVS", "$(V)"),
    ("BMI", "$(N)"),
    ("BLT", "$(N)!=$(V)"),
    ("BLE", "$(Z) || ($(N)!=$(V))"),
];

// Read and write selections differ for code 1. Code 0 reads a widened A
// with high byte 0x10, but writes only A. DP and PC are deliberately
// excluded: the former is not fully characterized and the latter needs
// dedicated control-flow semantics.
const TRANSFER_REGISTERS: [TransferRegister; 6] = [
    (0, "A", "zext(A)|0x1000", 1),
    (1, "B", "D", 1),
    (2, "X", "X", 2),
    (3, "Y", "Y", 2),
    (5, "U", "U", 2),
    (6, "S", "S", 2),
];

type TransferRegister = (u8, &'static str, &'static str, u8);
type ByteOp = (u8, &'static str, &'static str);
type WordOp = (u8, &'static str, &'static str, &'static str);

fn binary(reg: &str, val: &str, action: &str) -> String {
    let operator = match action {
        "and" => "&",
        "xor" => "^",
        "or" => "|",
        _ => return format!("{action}({reg},{val});"),
    };
    format!("{reg}={reg}{operator}{val}; nz({reg}); $(V)=0;")
}

fn instruction(
    output: &mut String,
    descriptor: (u8, &str, &str),
    body: &str,
) -> Result<(), String> {
    let (opcode, name, operand) = descriptor;
    writeln!(
        output,
        ":{name} {operand} is op=0x{opcode:02x}; {operand} {{ {body} }}"
    )
    .map_err(|error| error.to_string())
}

fn change_register(register: &str, kind: &str) -> String {
    match kind {
        "increment" => format!("{register}={register}+1;"),
        "increment2" => format!("{register}={register}+2;"),
        "decrement" => format!("{register}={register}-1;"),
        "decrement2" => format!("{register}={register}-2;"),
        _ => String::new(),
    }
}

const fn index_destination(indirect: u8) -> &'static str {
    if indirect == 0 {
        "addr"
    } else {
        "*:2 addr"
    }
}

fn offset_modes(
    output: &mut String,
    base: u8,
    register: &str,
) -> Result<(), String> {
    for (offset, operand, pre, address, post) in [
        (0u8, "", "", register, "increment"),
        (1, "", "", register, "increment2"),
        (2, "", "decrement", register, ""),
        (3, "", "decrement2", register, ""),
        (4, "; simm8", "", "byte", ""),
        (5, "; simm16", "", "word", ""),
        (6, "", "", register, ""),
    ] {
        let addr = match address {
            "byte" => format!("{register}+sext(simm8:1)"),
            "word" => format!("{register}+simm16"),
            _ => String::from(address),
        };
        let disp = match offset {
            0 => format!("{register}^\"+\""),
            1 => format!("{register}^\"++\""),
            2 => format!("\"-\"^{register}"),
            3 => format!("\"--\"^{register}"),
            4 => format!("simm8,{register}"),
            5 => format!("simm16,{register}"),
            _ => String::from(register),
        };
        let before = change_register(register, pre);
        let after = change_register(register, post);
        for indirect in [0u8, 8] {
            let mode = base | offset | indirect;
            let dest = index_destination(indirect);
            let display = if indirect == 0 {
                disp.clone()
            } else {
                format!("\"[\"{disp}\"]\"")
            };
            writeln!(
                output,
                "EA: {display} is postbyte=0x{mode:02x}{operand} & {register}"
            )
            .map_err(|error| error.to_string())?;
            writeln!(
                output,
                "{{ {before} local addr:2={addr}; {after} export {dest}; }}"
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn accumulator_modes(
    output: &mut String,
    base: u8,
    register: &str,
) -> Result<(), String> {
    for (offset, value) in [(0u8, "A"), (1, "B"), (7, "D")] {
        let addr = if value == "D" {
            format!("{register}+D")
        } else {
            format!("{register}+sext({value})")
        };
        for indirect in [0u8, 8] {
            let mode = base | 0x80 | offset | indirect;
            let dest = index_destination(indirect);
            let display = if indirect == 0 {
                format!("{value},{register}")
            } else {
                format!("\"[\"{value},{register}\"]\"")
            };
            writeln!(output, "EA: {display} is postbyte=0x{mode:02x} &")
                .map_err(|error| error.to_string())?;
            writeln!(output, "{value} & {register} {{ local addr:2={addr};")
                .map_err(|error| error.to_string())?;
            writeln!(output, "export {dest}; }}")
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn addressing(output: &mut String) -> Result<(), String> {
    for (base, register) in REGISTERS {
        offset_modes(output, base, register)?;
        accumulator_modes(output, base, register)?;
    }
    Ok(())
}

fn arithmetic(output: &mut String) -> Result<(), String> {
    for (base, name, action) in BYTE_OPS {
        for (offset, reg) in [(0u8, "A"), (1, "B")] {
            let mnemonic = format!("{name}{reg}");
            for (mode, operand) in [(0u8, "I8"), (2, "M8")] {
                instruction(
                    output,
                    (base | offset | mode, &mnemonic, operand),
                    &binary(reg, operand, action),
                )?;
            }
        }
    }
    for (base, mnemonic, register, action) in WORD_OPS {
        for (mode, operand) in [(0u8, "I16"), (1, "M16")] {
            instruction(
                output,
                (base | mode, mnemonic, operand),
                &binary(register, operand, action),
            )?;
        }
    }
    for (opcode, reg, operand) in [
        (0x3a, "A", "M8"),
        (0x3b, "B", "M8"),
        (0x58, "D", "M16"),
        (0x59, "X", "M16"),
        (0x5a, "Y", "M16"),
        (0x5b, "U", "M16"),
        (0x5c, "S", "M16"),
    ] {
        instruction(
            output,
            (opcode, &format!("ST{reg}"), operand),
            &format!("{operand}={reg}; nz({reg}); $(V)=0;"),
        )?;
    }
    Ok(())
}

fn branches(output: &mut String) -> Result<(), String> {
    for (position, (name, condition)) in BRANCHES.into_iter().enumerate() {
        let index =
            u8::try_from(position).map_err(|error| error.to_string())?;
        let offset = (index & 7) | ((index & 8) << 1u8);
        for (long, operand) in [(0u8, "R8"), (8, "R16")] {
            let mnemonic = if long == 0 {
                String::from(name)
            } else {
                format!("L{name}")
            };
            let body = match condition {
                "0" => String::new(),
                "1" => format!("goto {operand};"),
                _ => format!("if ({condition}) goto {operand};"),
            };
            instruction(
                output,
                (0x60 | offset | long, &mnemonic, operand),
                &body,
            )?;
        }
    }
    Ok(())
}

fn unary(output: &mut String) -> Result<(), String> {
    for (base, name, action) in [
        (0x80u8, "CLR", "clear_value"),
        (0x83, "COM", "complement"),
        (0x86, "NEG", "negate8"),
        (0x89, "INC", "inc8"),
        (0x8c, "DEC", "dec8"),
        (0x90, "TST", "test_value"),
        (0x93, "LSR", "lsr"),
        (0x96, "ROR", "ror8"),
        (0x99, "ASR", "asr"),
        (0x9c, "ASL", "asl"),
        (0xa0, "ROL", "rol8"),
    ] {
        for (offset, register) in [(0u8, "A"), (1, "B")] {
            let opcode = base.saturating_add(offset);
            writeln!(output, ":{name}{register} is op=0x{opcode:02x}")
                .map_err(|error| error.to_string())?;
            writeln!(output, "{{ {action}({register}); }}")
                .map_err(|error| error.to_string())?;
        }
        instruction(
            output,
            (base.saturating_add(2), name, "M8"),
            &format!("{action}(M8);"),
        )?;
    }
    for (opcode, name, action) in [
        (0xa3, "LSRW", "lsr"),
        (0xa4, "RORW", "ror16"),
        (0xa5, "ASRW", "asr"),
        (0xa6, "ASLW", "asl"),
        (0xa7, "ROLW", "rol16"),
    ] {
        instruction(output, (opcode, name, "M16"), &format!("{action}(M16);"))?;
    }
    for (base, name, action) in [
        (0xc2, "CLR", "clear_value"),
        (0xc4, "NEG", "negate16"),
        (0xc6, "INC", "inc16"),
        (0xc8, "DEC", "dec16"),
        (0xca, "TST", "test_value"),
    ] {
        writeln!(output, ":{name}D is op=0x{base:02x} {{ {action}(D); }}")
            .map_err(|error| error.to_string())?;
        instruction(
            output,
            (base | 1, &format!("{name}W"), "M16"),
            &format!("{action}(M16);"),
        )?;
    }
    Ok(())
}

fn repeat_word_shifts(output: &mut String) -> Result<(), String> {
    for (opcode, mnemonic, action) in [
        (0xb8u8, "LSRDI", "lsr"),
        (0xba, "RORDI", "ror16"),
        (0xbc, "ASRDI", "asr"),
        (0xbe, "ASLDI", "asl"),
        (0xc0, "ROLDI", "rol16"),
    ] {
        for (offset, operand, value, count) in
            [(0u8, "I8", "D", "I8"), (1, "M16", "M16", "A")]
        {
            let body = format!(
                "local count:1={count}&0xf; local value:2={value}; \
                 <again> if (count==0) goto <done>; {action}(value); \
                 count=count-1; goto <again>; <done> {value}=value;"
            );
            instruction(output, (opcode | offset, mnemonic, operand), &body)?;
        }
    }
    Ok(())
}

const fn stack_registers(other: &str) -> [(&str, u8); 8] {
    [
        ("CC", 1),
        ("A", 1),
        ("B", 1),
        ("DP", 1),
        ("X", 2),
        ("Y", 2),
        (other, 2),
        ("PC", 2),
    ]
}

fn stack_body(
    mask: u8,
    stack: &str,
    other: &str,
    pull: bool,
) -> Result<String, String> {
    let mut body = String::new();
    let regs = stack_registers(other);
    let order: Vec<_> = if pull {
        regs.iter().enumerate().collect()
    } else {
        regs.iter().enumerate().rev().collect()
    };
    for (bit, &(register, size)) in order {
        let flag = 1u8
            .checked_shl(u32::try_from(bit).map_err(|error| error.to_string())?)
            .ok_or("stack mask")?;
        if mask & flag == 0 {
            continue;
        }
        let value = if register == "PC" {
            "inst_next"
        } else {
            register
        };
        if !pull {
            writeln!(body, "{stack}={stack}-{size}; *:{size} {stack}={value};")
                .map_err(|error| error.to_string())?;
        } else if register == "PC" {
            writeln!(body, "local dest:2=*:2 {stack}; {stack}={stack}+2;")
                .map_err(|error| error.to_string())?;
            let transfer = if stack == "S" {
                "return"
            } else {
                "goto"
            };
            writeln!(body, "{transfer} [dest];")
                .map_err(|error| error.to_string())?;
        } else {
            writeln!(
                body,
                "{register}=*:{size} {stack}; {stack}={stack}+{size};"
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(body)
}

fn stacks(output: &mut String) -> Result<(), String> {
    for (opcode, name, stack, other, pull) in [
        (0x0cu8, "PSHS", "S", "U", false),
        (0x0d, "PSHU", "U", "S", false),
        (0x0e, "PULS", "S", "U", true),
        (0x0f, "PULU", "U", "S", true),
    ] {
        for mask in 0u8..=u8::MAX {
            let body = stack_body(mask, stack, other, pull)?;
            writeln!(
                output,
                ":{name} \"0x{mask:02x}\" is op=0x{opcode:02x}; \
                 postbyte=0x{mask:02x}"
            )
            .map_err(|error| error.to_string())?;
            writeln!(output, "{{ {body} }}")
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn transfer_write(register: &str, size: u8, value: &str) -> String {
    if size == 1 {
        format!("{register}={value}:1;")
    } else {
        format!("{register}={value};")
    }
}

fn transfer_pair(
    output: &mut String,
    source_reg: TransferRegister,
    target_reg: TransferRegister,
    operation: (u8, &str),
) -> Result<(), String> {
    let (source, src, src_read, src_size) = source_reg;
    let (target, dst, dst_read, dst_size) = target_reg;
    let (mode, mnemonic) = operation;
    let write_dst = transfer_write(dst, dst_size, "source");
    let exchange = if mode == 0 {
        format!(
            "local target:2={dst_read}; {}",
            transfer_write(src, src_size, "target")
        )
    } else {
        String::new()
    };
    // The opcode low bit and argument bit 3 are aliases.
    // Argument bit 7 selects the operation.
    for opcode in [0x3eu8, 0x3f] {
        for alias in [0u8, 8] {
            let post = mode | (target << 4u8) | source | alias;
            writeln!(
                output,
                ":{mnemonic} \"{src},{dst}\" is op=0x{opcode:02x}; \
                 postbyte=0x{post:02x}"
            )
            .map_err(|error| error.to_string())?;
            writeln!(
                output,
                "{{ local source:2={src_read}; {exchange} {write_dst} }}"
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn transfers(output: &mut String) -> Result<(), String> {
    for source in TRANSFER_REGISTERS {
        for target in TRANSFER_REGISTERS {
            for (mode, mnemonic) in [(0u8, "EXG"), (0x80, "TFR")] {
                transfer_pair(output, source, target, (mode, mnemonic))?;
            }
        }
    }
    Ok(())
}

pub(super) fn sleigh() -> Result<String, String> {
    let mut output = String::from(BASE);
    addressing(&mut output)?;
    arithmetic(&mut output)?;
    branches(&mut output)?;
    unary(&mut output)?;
    repeat_word_shifts(&mut output)?;
    stacks(&mut output)?;
    transfers(&mut output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn generator_uses_konami_opcodes_without_a_catchall() -> Result<(), String>
    {
        let text = sleigh()?;
        for expected in [
            ":LDA I8 is op=0x10;",
            ":LDA M8 is op=0x12;",
            ":LDX I16 is op=0x42;",
            ":BRA R8 is op=0x60;",
            ":LBEQ R16 is op=0x7b;",
            ":LEAX EA is op=0x08; EA { X=EA; $(Z)=X==0; }
:LEAY EA is op=0x09; EA { Y=EA; $(Z)=Y==0; }
:LEAU EA is op=0x0a; EA { U=EA; }
:LEAS EA is op=0x0b; EA { S=EA; }
:JMP ABS is op=0xa8; ABS { goto ABS; }
:JSR ABS is op=0xa9; ABS { pushret(); call ABS; }
:JMP EA is op=0xa8; EA { goto [EA]; }
:JSR EA is op=0xa9; EA { pushret(); call [EA]; }
:DBJNZ R8 is op=0xac; R8 {
    $(V)=B==0x80; B=B-1; nz(B); if ($(Z)==0) goto R8;
}
:DXJNZ R8 is op=0xad; R8 {
    $(V)=X==0x8000; X=X-1; nz(X); if ($(Z)==0) goto R8;
}
:RTS is op=0x8f",
        ] {
            if !text.contains(expected) {
                return Err(format!("missing constructor: {expected}"));
            }
        }
        if text.contains("op=0x00") || text.contains("op=0xff") {
            return Err(String::from("unsupported instruction admitted"));
        }
        Ok(())
    }

    #[test]
    fn transfers_preserve_asymmetric_register_selection() -> Result<(), String>
    {
        let text = sleigh()?;
        for expected in [
            ":EXG \"A,B\" is op=0x3e; postbyte=0x10",
            ":EXG \"A,B\" is op=0x3f; postbyte=0x18",
            ":TFR \"A,B\" is op=0x3e; postbyte=0x90",
            "local source:2=zext(A)|0x1000;",
            "local target:2=D; A=target:1;",
            "B=source:1;",
            ":RTI is op=0x9f",
            "if ((CC&0x80)==0) goto <pc>;",
            ":BMOVE is op=0xb6",
            "if (U==0) goto <done>;",
        ] {
            if !text.contains(expected) {
                return Err(format!("missing processor behavior: {expected}"));
            }
        }
        if text.contains(":TFR \"A,DP\"") || text.contains(":EXG \"PC,A\"") {
            return Err(String::from("unsupported transfer admitted"));
        }
        Ok(())
    }

    #[test]
    fn documented_opcode_values_and_repeat_counts_are_admitted()
    -> Result<(), String> {
        let text = sleigh()?;
        let opcodes = text
            .split("op=0x")
            .skip(1)
            .map(|suffix| {
                suffix
                    .get(..2)
                    .ok_or_else(|| String::from("missing opcode"))
                    .and_then(|digits| {
                        u8::from_str_radix(digits, 16)
                            .map_err(|error| error.to_string())
                    })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if opcodes.len() != 197
            || !(0xb0..=0xd0).all(|opcode| opcodes.contains(&opcode))
        {
            return Err(String::from("documented opcode coverage changed"));
        }
        for expected in [
            ":DAA is op=0xb1",
            ":LMUL is op=0xb4",
            ":DIVX is op=0xb5",
            ":ABSD is op=0xce",
            ":BSETW is op=0xd0",
            ":LSRDI I8 is op=0xb8",
            ":ROLDI M16 is op=0xc1",
            "local count:1=I8&0xf;",
            "local count:1=A&0xf;",
            "if (count==0) goto <done>;",
            "local result:4=zext(X)*zext(Y);",
        ] {
            if !text.contains(expected) {
                return Err(format!("missing processor behavior: {expected}"));
            }
        }
        Ok(())
    }
}
