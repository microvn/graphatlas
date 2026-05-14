<?php

namespace App\Service;

use App\Entity\User;
use App\Repository\UserRepository;
use App\Util\{Logger, Cache};
use App\Auth\AuthService as AuthSvc;
use function strlen;
use const PHP_INT_MAX;

class UserService
{
    #[Required]
    public UserRepository $repo;

    #[ORM\Column(type: 'string')]
    public string $name;

    #[Required]
    #[Lazy]
    public Logger $logger;

    public function getUser(int $id): ?User
    {
        return $this->repo->findById($id);
    }

    public function register(string $name): User
    {
        $user = new User(PHP_INT_MAX, $name);
        Cache::warm($user);
        AuthSvc::grantDefault($user);
        return $this->repo->save($user);
    }

    public function nullsafeLookup(?UserRepository $maybe, int $id): ?User
    {
        return $maybe?->findById($id);
    }

    public function bareCall(string $s): int
    {
        return strlen($s);
    }
}
