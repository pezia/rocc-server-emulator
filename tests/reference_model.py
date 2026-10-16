"""Independent, deliberately small model of documented base CPU instructions.

This code never runs/imports the Rust emulator. It models no peripherals or ROCC
extensions. Carry rotation uses bit lists; memory is a sparse Python dictionary.
"""
MODEL_VERSION=1

def evaluate(c):
    regs=list(c['ac']); carry=c['carry']; pc=c['pc']; mask=(1<<c['address_bits'])-1
    memory=dict(c['memory']); reason='step_limit'
    def read(a):
        if a not in memory:raise LookupError('uninitialized_memory')
        return memory[a]
    try:
        for _ in range(c['steps']):
            word=read(pc); nxt=(pc+1)&mask
            if word&0x8000:
                if c['extended'] and word&15==8:raise LookupError('unsupported_rocc_instruction')
                src=(word>>13)&3; dst=(word>>11)&3; operation=(word>>8)&7
                k=[carry,0,1,1-carry][(word>>4)&3]; a=regs[src]; b=regs[dst]
                # Separately express low-word arithmetic and the carry accumulator.
                arithmetic=[65535-a,65536-a,a,a+1,65535-a+b,65536-a+b,a+b,a&b][operation]
                high=(k+(arithmetic//65536))%2
                low=arithmetic%65536
                bits=[high]+[(low>>i)&1 for i in range(15,-1,-1)]
                shift=(word>>6)&3
                if shift==1:bits=bits[1:]+bits[:1]
                elif shift==2:bits=bits[-1:]+bits[:-1]
                elif shift==3:bits=bits[:1]+bits[9:]+bits[1:9]
                candidate=0
                for bit in bits[1:]:candidate=candidate*2+bit
                candidate_carry=bits[0]
                skip=[False,True,candidate_carry==0,candidate_carry==1,candidate==0,candidate!=0,candidate_carry==0 or candidate==0,candidate_carry==1 and candidate!=0][word&7]
                if skip:nxt=(nxt+1)&mask
                if not word&8:regs[dst]=candidate;carry=candidate_carry
            elif word<0x6000:
                op=word>>11;mode=(word>>8)&3;disp=word&255
                base=[0,pc,regs[2],regs[3]][mode]
                address=(base+(disp if mode==0 or disp<128 else disp-256))&mask
                if word&0x400:
                    for _ in range(256):
                        pointer=read(address);target=pointer
                        if 16<=address<32:
                            target=(pointer+(1 if address<24 else -1))&65535;memory[address]=target
                        address=target&mask
                        if mask==65535 or not pointer&0x8000:break
                    else:raise LookupError('indirect_limit')
                if op==0:nxt=address
                elif op==1:regs[3]=nxt;nxt=address
                elif op in (2,3):
                    value=(read(address)+(1 if op==2 else -1))&65535;memory[address]=value
                    if value==0:nxt=(nxt+1)&mask
                elif op<8:regs[op&3]=read(address)
                else:memory[address]=regs[op&3]
            elif word&0xe7ff==0x663f:
                pc=nxt;reason='halt';break
            else:raise ValueError('reference corpus contains unsupported peripheral instruction')
            pc=nxt
    except LookupError as error:reason=str(error)
    state=dict(pc=pc,ac=regs,carry=carry)
    if c['extended']:state['bank']=c.get('bank',0)
    return dict(state=state,reason=reason,memory=[list(item) for item in sorted(memory.items())])
