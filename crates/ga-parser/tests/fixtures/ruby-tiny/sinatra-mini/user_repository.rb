require_relative 'user'

module App
  class UserRepository
    def initialize(store)
      @store = store
    end

    def find_by_id(id)
      User.find(id)
    end

    def save(user)
      @store.put(user)
    end

    # AS-014 candidate — dynamic method definition (Tools-C11 confidence ≤0.6).
    define_method(:reload) do
      @store.refresh
    end
  end
end
