package com.modrinth.theseus.agent.transformers;

import java.util.ListIterator;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.ClassNode;
import org.objectweb.asm.tree.FrameNode;
import org.objectweb.asm.tree.InsnList;
import org.objectweb.asm.tree.InsnNode;
import org.objectweb.asm.tree.JumpInsnNode;
import org.objectweb.asm.tree.LabelNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.VarInsnNode;

/**
 * Engancha {@code ClientPacketListener} para el comando {@code /strata}: corta {@code sendCommand} cuando el comando es
 * de Strata y lo registra cada vez que llega la lista de comandos del servidor.
 *
 * <p>Si falta cualquiera de los dos métodos no se toca nada, para que nunca quede un comando a medias.
 */
public final class ClientCommandsTransformer extends ClassNodeTransformer {
    private static final String COMMANDS = "com/modrinth/theseus/StrataCommands";
    private static final String SEND_COMMAND_DESC = "(Ljava/lang/String;)V";
    private static final String HANDLE_COMMANDS_DESC =
            "(Lnet/minecraft/network/protocol/game/ClientboundCommandsPacket;)V";

    @Override
    protected boolean transform(ClassNode classNode) {
        MethodNode send = null;
        MethodNode handle = null;
        for (final MethodNode method : classNode.methods) {
            if (method.name.equals("sendCommand") && method.desc.equals(SEND_COMMAND_DESC)) {
                if (send != null) {
                    return false;
                }
                send = method;
            } else if (method.name.equals("handleCommands") && method.desc.equals(HANDLE_COMMANDS_DESC)) {
                if (handle != null) {
                    return false;
                }
                handle = method;
            }
        }
        if (send == null || handle == null || (send.access & Opcodes.ACC_STATIC) != 0) {
            return false;
        }

        // if (StrataCommands.intercept(command)) return;
        final LabelNode proceed = new LabelNode();
        final InsnList guard = new InsnList();
        guard.add(new VarInsnNode(Opcodes.ALOAD, 1));
        guard.add(new MethodInsnNode(Opcodes.INVOKESTATIC, COMMANDS, "intercept", "(Ljava/lang/String;)Z"));
        guard.add(new JumpInsnNode(Opcodes.IFEQ, proceed));
        guard.add(new InsnNode(Opcodes.RETURN));
        guard.add(proceed);
        if (needsStackMap(classNode)) {
            guard.add(new FrameNode(Opcodes.F_SAME, 0, null, 0, null));
        }
        send.instructions.insert(guard);

        // Antes de cada return de handleCommands: StrataCommands.install(this)
        final ListIterator<AbstractInsnNode> it = handle.instructions.iterator();
        while (it.hasNext()) {
            final AbstractInsnNode insn = it.next();
            if (insn.getOpcode() == Opcodes.RETURN) {
                it.previous();
                it.add(new VarInsnNode(Opcodes.ALOAD, 0));
                it.add(new MethodInsnNode(Opcodes.INVOKESTATIC, COMMANDS, "install", "(Ljava/lang/Object;)V"));
                it.next();
            }
        }
        return true;
    }
}
