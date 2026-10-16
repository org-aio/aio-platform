// 只对明确观测到的失败回滚；未结束、取消或进程中断不当作失败样本。
export function invoke(input: any) {
    if (input.minimum_completed < 1 || input.failure_ratio <= 0 || input.failure_ratio > 1) {
        throw new Error('观察阈值无效');
    }
    const head = call_capability('/v1/runtime/enhancements/head', {});
    if (head.revision !== input.revision || head.generation !== input.expected_generation) {
        throw new Error('观察版本已变化，必须重新审阅');
    }
    const observations = call_capability('/v1/runtime/enhancements/observe', { revision: head.revision, generation: head.generation });
    const sample = observations.find((entry: any) => entry.path === input.path);
    if (!sample || sample.succeeded + sample.failed < input.minimum_completed ||
        sample.failed / (sample.succeeded + sample.failed) < input.failure_ratio) {
        return { rolled_back: false, head };
    }
    const restored = call_capability('/v1/runtime/enhancements/rollback', {
        revision: input.previous_revision,
        expected_generation: head.generation,
    });
    return { rolled_back: true, head: restored };
}
