// 策略本身也是普通增强；模型只返回源码，路径、依赖和验收样例由请求明确提供。
export function invoke(input: any) {
    const head = call_capability('/v1/runtime/enhancements/head', {});
    if (head.generation !== input.expected_generation) {
        throw new Error('活动版本已变化，必须重新审阅');
    }
    const current = call_capability('/v1/runtime/enhancements/package', { revision: head.revision });
    const target = input.target;
    const available = call_capability('/v1/runtime/enhancements/catalog', { paths: target.capabilities });
    if (available.length !== target.capabilities.length) {
        throw new Error('请求依赖的能力未装配或未授权');
    }
    const generated = call_capability('/v1/model/enhancement/source', {
        requirement: input.requirement,
        target,
        available,
    });
    const candidate = { ...target, source: generated.source };
    const previous = current.enhancements.findIndex((entry: any) => entry.path === target.path);
    if (previous < 0) {
        current.enhancements.push(candidate);
    } else {
        current.enhancements[previous] = candidate;
    }
    const proposed = call_capability('/v1/runtime/enhancements/propose', current);
    return call_capability('/v1/runtime/enhancements/activate', {
        revision: proposed.revision,
        expected_generation: head.generation,
    });
}
