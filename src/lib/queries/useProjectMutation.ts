import { useMutation, type MutateOptions, type UseMutationOptions } from '@tanstack/react-query'
import { assertProjectSession, isProjectSession, projectSessionEpoch } from '../projectSession'

/** Capture before onMutate can await, and never adopt a newer project for the same mutation. */
export function useProjectMutation<T, E = Error, V = void, C = unknown>(
  options: UseMutationOptions<T, E, V, C> & {
    mutationFn: NonNullable<UseMutationOptions<T, E, V, C>['mutationFn']>
  },
) {
  type Input = { value: V; epoch: number }
  const callbacks = (value: MutateOptions<T, E, V, C>): MutateOptions<T, E, Input, C> => ({
    onSuccess: (data, input, context, mutation) => {
      if (isProjectSession(input.epoch)) value.onSuccess?.(data, input.value, context, mutation)
    },
    onError: (error, input, context, mutation) => {
      if (isProjectSession(input.epoch)) value.onError?.(error, input.value, context, mutation)
    },
    onSettled: (data, error, input, context, mutation) => {
      if (isProjectSession(input.epoch))
        value.onSettled?.(data, error, input.value, context, mutation)
    },
  })
  const mutation = useMutation<T, E, Input, C>({
    ...options,
    mutationFn: (input, context) => {
      assertProjectSession(input.epoch)
      return options.mutationFn(input.value, context)
    },
    onMutate: (input, context) => {
      assertProjectSession(input.epoch)
      return options.onMutate?.(input.value, context) as C | Promise<C>
    },
    onSuccess: (data, input, context, mutation) => {
      if (isProjectSession(input.epoch))
        return options.onSuccess?.(data, input.value, context, mutation)
    },
    onError: (error, input, context, mutation) => {
      if (isProjectSession(input.epoch))
        return options.onError?.(error, input.value, context, mutation)
    },
    onSettled: (data, error, input, context, mutation) => {
      if (isProjectSession(input.epoch))
        return options.onSettled?.(data, error, input.value, context, mutation)
    },
  })
  return {
    ...mutation,
    variables: mutation.variables?.value,
    mutate: (value: V, perCall: MutateOptions<T, E, V, C> = {}) =>
      mutation.mutate({ value, epoch: projectSessionEpoch() }, callbacks(perCall)),
    mutateAsync: (value: V, perCall: MutateOptions<T, E, V, C> = {}) =>
      mutation.mutateAsync({ value, epoch: projectSessionEpoch() }, callbacks(perCall)),
  }
}
