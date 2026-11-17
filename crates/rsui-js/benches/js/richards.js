// Richards: an operating-system task scheduler simulation, after Martin Richards' benchmark as it appears in the V8 and
// Octane suites. Lots of small method calls on a few classes, linked lists, and field updates.
const COUNT = 1000, EXPECTED_QUEUE_COUNT = 2322, EXPECTED_HOLD_COUNT = 928;
const ID_IDLE = 0, ID_WORKER = 1, ID_HANDLER_A = 2, ID_HANDLER_B = 3, ID_DEVICE_A = 4, ID_DEVICE_B = 5, NUMBER_OF_IDS = 6;
const KIND_DEVICE = 0, KIND_WORK = 1;
const DATA_SIZE = 4;
const STATE_RUNNING = 0, STATE_RUNNABLE = 1, STATE_SUSPENDED = 2, STATE_HELD = 4;
const STATE_SUSPENDED_RUNNABLE = STATE_SUSPENDED | STATE_RUNNABLE, STATE_NOT_HELD = ~STATE_HELD;

function runRichards() {
    const scheduler = new Scheduler();
    scheduler.addIdleTask(ID_IDLE, 0, null, COUNT);
    let queue = new Packet(null, ID_WORKER, KIND_WORK);
    queue = new Packet(queue, ID_WORKER, KIND_WORK);
    scheduler.addWorkerTask(ID_WORKER, 1000, queue);
    queue = new Packet(null, ID_DEVICE_A, KIND_DEVICE);
    queue = new Packet(queue, ID_DEVICE_A, KIND_DEVICE);
    queue = new Packet(queue, ID_DEVICE_A, KIND_DEVICE);
    scheduler.addHandlerTask(ID_HANDLER_A, 2000, queue);
    queue = new Packet(null, ID_DEVICE_B, KIND_DEVICE);
    queue = new Packet(queue, ID_DEVICE_B, KIND_DEVICE);
    queue = new Packet(queue, ID_DEVICE_B, KIND_DEVICE);
    scheduler.addHandlerTask(ID_HANDLER_B, 3000, queue);
    scheduler.addDeviceTask(ID_DEVICE_A, 4000, null);
    scheduler.addDeviceTask(ID_DEVICE_B, 5000, null);
    scheduler.schedule();
    if (scheduler.queueCount !== EXPECTED_QUEUE_COUNT || scheduler.holdCount !== EXPECTED_HOLD_COUNT) {
        throw new Error(`richards: queue ${scheduler.queueCount} hold ${scheduler.holdCount}`);
    }
    return scheduler.queueCount + scheduler.holdCount;
}

class Scheduler {
    constructor() {
        this.queueCount = 0;
        this.holdCount = 0;
        this.blocks = new Array(NUMBER_OF_IDS);
        this.list = null;
        this.currentTcb = null;
        this.currentId = null;
    }
    addIdleTask(id, priority, queue, count) { this.addRunningTask(id, priority, queue, new IdleTask(this, 1, count)); }
    addWorkerTask(id, priority, queue) { this.addTask(id, priority, queue, new WorkerTask(this, ID_HANDLER_A, 0)); }
    addHandlerTask(id, priority, queue) { this.addTask(id, priority, queue, new HandlerTask(this)); }
    addDeviceTask(id, priority, queue) { this.addTask(id, priority, queue, new DeviceTask(this)); }
    addRunningTask(id, priority, queue, task) { this.addTask(id, priority, queue, task); this.currentTcb.setRunning(); }
    addTask(id, priority, queue, task) {
        this.currentTcb = new TaskControlBlock(this.list, id, priority, queue, task);
        this.list = this.currentTcb;
        this.blocks[id] = this.currentTcb;
    }
    schedule() {
        this.currentTcb = this.list;
        while (this.currentTcb !== null) {
            if (this.currentTcb.isHeldOrSuspended()) {
                this.currentTcb = this.currentTcb.link;
            } else {
                this.currentId = this.currentTcb.id;
                this.currentTcb = this.currentTcb.run();
            }
        }
    }
    release(id) {
        const tcb = this.blocks[id];
        if (tcb === null) return tcb;
        tcb.markAsNotHeld();
        if (tcb.priority > this.currentTcb.priority) return tcb;
        return this.currentTcb;
    }
    holdCurrent() {
        this.holdCount++;
        this.currentTcb.markAsHeld();
        return this.currentTcb.link;
    }
    suspendCurrent() {
        this.currentTcb.markAsSuspended();
        return this.currentTcb;
    }
    queue(packet) {
        const t = this.blocks[packet.id];
        if (t === null) return t;
        this.queueCount++;
        packet.link = null;
        packet.id = this.currentId;
        return t.checkPriorityAdd(this.currentTcb, packet);
    }
}

class TaskControlBlock {
    constructor(link, id, priority, queue, task) {
        this.link = link;
        this.id = id;
        this.priority = priority;
        this.queue = queue;
        this.task = task;
        this.state = queue === null ? STATE_SUSPENDED : STATE_SUSPENDED_RUNNABLE;
    }
    setRunning() { this.state = STATE_RUNNING; }
    markAsNotHeld() { this.state = this.state & STATE_NOT_HELD; }
    markAsHeld() { this.state = this.state | STATE_HELD; }
    isHeldOrSuspended() { return (this.state & STATE_HELD) !== 0 || this.state === STATE_SUSPENDED; }
    markAsSuspended() { this.state = this.state | STATE_SUSPENDED; }
    markAsRunnable() { this.state = this.state | STATE_RUNNABLE; }
    run() {
        let packet;
        if (this.state === STATE_SUSPENDED_RUNNABLE) {
            packet = this.queue;
            this.queue = packet.link;
            this.state = this.queue === null ? STATE_RUNNING : STATE_RUNNABLE;
        } else {
            packet = null;
        }
        return this.task.run(packet);
    }
    checkPriorityAdd(task, packet) {
        if (this.queue === null) {
            this.queue = packet;
            this.markAsRunnable();
            if (this.priority > task.priority) return this;
        } else {
            this.queue = packet.addTo(this.queue);
        }
        return task;
    }
}

class IdleTask {
    constructor(scheduler, v1, count) { this.scheduler = scheduler; this.v1 = v1; this.count = count; }
    run(packet) {
        this.count--;
        if (this.count === 0) return this.scheduler.holdCurrent();
        if ((this.v1 & 1) === 0) {
            this.v1 = this.v1 >> 1;
            return this.scheduler.release(ID_DEVICE_A);
        }
        this.v1 = (this.v1 >> 1) ^ 0xD008;
        return this.scheduler.release(ID_DEVICE_B);
    }
}

class DeviceTask {
    constructor(scheduler) { this.scheduler = scheduler; this.v1 = null; }
    run(packet) {
        if (packet === null) {
            if (this.v1 === null) return this.scheduler.suspendCurrent();
            const v = this.v1;
            this.v1 = null;
            return this.scheduler.queue(v);
        }
        this.v1 = packet;
        return this.scheduler.holdCurrent();
    }
}

class WorkerTask {
    constructor(scheduler, v1, v2) { this.scheduler = scheduler; this.v1 = v1; this.v2 = v2; }
    run(packet) {
        if (packet === null) return this.scheduler.suspendCurrent();
        this.v1 = this.v1 === ID_HANDLER_A ? ID_HANDLER_B : ID_HANDLER_A;
        packet.id = this.v1;
        packet.a1 = 0;
        for (let i = 0; i < DATA_SIZE; i++) {
            this.v2++;
            if (this.v2 > 26) this.v2 = 1;
            packet.a2[i] = this.v2;
        }
        return this.scheduler.queue(packet);
    }
}

class HandlerTask {
    constructor(scheduler) { this.scheduler = scheduler; this.v1 = null; this.v2 = null; }
    run(packet) {
        if (packet !== null) {
            if (packet.kind === KIND_WORK) this.v1 = packet.addTo(this.v1);
            else this.v2 = packet.addTo(this.v2);
        }
        if (this.v1 !== null) {
            const count = this.v1.a1;
            let v;
            if (count < DATA_SIZE) {
                if (this.v2 !== null) {
                    v = this.v2;
                    this.v2 = this.v2.link;
                    v.a1 = this.v1.a2[count];
                    this.v1.a1 = count + 1;
                    return this.scheduler.queue(v);
                }
            } else {
                v = this.v1;
                this.v1 = this.v1.link;
                return this.scheduler.queue(v);
            }
        }
        return this.scheduler.suspendCurrent();
    }
}

class Packet {
    constructor(link, id, kind) {
        this.link = link;
        this.id = id;
        this.kind = kind;
        this.a1 = 0;
        this.a2 = new Array(DATA_SIZE);
    }
    addTo(queue) {
        this.link = null;
        if (queue === null) return this;
        let peek, next = queue;
        while ((peek = next.link) !== null) next = peek;
        next.link = this;
        return queue;
    }
}

let total = 0;
for (let i = 0; i < 120; i++) total += runRichards();
console.log('richards', total);
