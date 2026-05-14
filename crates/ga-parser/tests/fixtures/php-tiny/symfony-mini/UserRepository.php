<?php

namespace App\Repository;

use App\Entity\User;

interface UserRepository
{
    public function findById(int $id): ?User;
    public function save(User $user): User;
}
