require_relative 'user_repository'

module App
  class UserService
    def initialize(repo)
      @repo = repo
    end

    def login(id, password)
      user = @repo.find_by_id(id)
      return nil unless user
      user.authenticate(password) ? user : nil
    end

    def register(name, email)
      user = User.new(name, email)
      @repo.save(user)
      user
    end
  end
end
