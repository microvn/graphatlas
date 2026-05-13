module App
  class User < Base
    attr_accessor :name, :email

    def initialize(name, email)
      @name = name
      @email = email
    end

    def authenticate(password)
      check(password)
    end

    def self.find(id)
      Base.lookup(id)
    end
  end
end
